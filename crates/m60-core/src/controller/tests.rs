//! Controller tests against a fake Bluetooth stack with paused (virtual) time.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::*;
use crate::proto::{build_frame, checksum};
use crate::state::Resume;

enum Msg {
    Data(Vec<u8>),
    Closed,
}

#[derive(Default)]
struct Probe {
    written: Vec<Vec<u8>>,
    scans: Vec<Instant>,
    scanned_prefer: Vec<Option<String>>,
    connects: Vec<String>,
    closed: usize,
    inject: Option<mpsc::UnboundedSender<Msg>>,
    alive: Option<Arc<AtomicBool>>,
    /// The speaker is off / taken by another device: scans find nothing.
    offline: bool,
    /// What the fake speaker now answers to a read, after a change was written to it.
    remembered: HashMap<u8, Vec<u8>>,
    /// The fake speaker does not announce input changes (by default it does, like the real one).
    quiet_inputs: bool,
}

type Shared = Arc<Mutex<Probe>>;

fn rsp(cmd: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![p::RSP, 0xEC, cmd];
    f.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    f.extend_from_slice(payload);
    f.push(checksum(&f));
    f
}

/// What a healthy M60 answers to each initial read (the values the user's speaker had).
fn default_replies() -> HashMap<u8, Vec<u8>> {
    HashMap::from([
        (p::CMD_GET_VOLUME, rsp(0x66, &[0x10, 7])),
        (p::CMD_INPUT_GET, rsp(0x61, &[0x0F, 1])),
        (p::CMD_EQ_GET, rsp(0xD5, &[0])),
        (p::CMD_BACKLIGHT_GET, rsp(0x84, &[3, 1, 1])),
        (p::CMD_PROMPT_GET, rsp(0x86, &[2, 1])),
        (p::CMD_CUSTOM_EQ_GET, rsp(0x43, &hex_bytes(GAMING_43))),
        (p::CMD_FIRMWARE_GET, rsp(0xC6, &[2, 4, 1])),
        (p::CMD_NAME, rsp(0xC9, b"EDIFIER M60")),
    ])
}

struct FakeLink {
    probe: Shared,
    rx: mpsc::UnboundedReceiver<Msg>,
    tx: mpsc::UnboundedSender<Msg>,
    alive: Arc<AtomicBool>,
    replies: HashMap<u8, Vec<u8>>,
}

impl Link for FakeLink {
    async fn write(&mut self, data: &[u8]) -> Result<(), String> {
        let mut p = self.probe.lock().unwrap();
        p.written.push(data.to_vec());
        // like the real speaker, remember changes so later reads return them
        let changed = match (data[2], &data[5..data.len() - 1]) {
            (0x67, [v]) => Some((0x66, vec![0x10, *v])),
            (0x62, [0x0F, n]) => Some((0x61, vec![0x0F, *n])),
            (0xC4, [n]) => Some((0xD5, vec![*n])),
            _ => None,
        };
        if let Some((read, payload)) = changed {
            p.remembered.insert(read, rsp(read, &payload));
            // the real speaker announces the new input with a 0x61 notification
            if read == 0x61 && !p.quiet_inputs {
                let _ = self.tx.send(Msg::Data(rsp(0x61, &payload)));
            }
        }
        // power off: the speaker goes away until it is switched on again
        if data[2] == p::CMD_POWER_OFF {
            p.offline = true;
            let _ = self.tx.send(Msg::Closed);
        }
        if let Some(r) = p.remembered.get(&data[2]).or_else(|| self.replies.get(&data[2])) {
            let _ = self.tx.send(Msg::Data(r.clone()));
        }
        Ok(())
    }
    async fn next_data(&mut self) -> Option<Vec<u8>> {
        match self.rx.recv().await {
            Some(Msg::Data(d)) => Some(d),
            _ => {
                self.alive.store(false, Ordering::SeqCst);
                None
            }
        }
    }
    async fn is_connected(&mut self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
    async fn close(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        self.probe.lock().unwrap().closed += 1;
    }
}

#[derive(Clone)]
enum ConnectMode {
    Ok,
    Hang,
    Fail(&'static str),
}

struct FakeRadio {
    probe: Shared,
    adverts: Vec<Advert>,
    mode: ConnectMode,
    replies: HashMap<u8, Vec<u8>>,
}

impl Radio for FakeRadio {
    type Link = FakeLink;
    async fn scan(&mut self, _t: Duration, prefer: Option<&str>) -> Result<Vec<Advert>, String> {
        let mut p = self.probe.lock().unwrap();
        p.scans.push(Instant::now());
        p.scanned_prefer.push(prefer.map(str::to_string));
        Ok(if p.offline { Vec::new() } else { self.adverts.clone() })
    }
    async fn connect(&mut self, id: &str) -> Result<FakeLink, String> {
        self.probe.lock().unwrap().connects.push(id.to_string());
        match &self.mode {
            ConnectMode::Hang => std::future::pending().await,
            ConnectMode::Fail(e) => Err((*e).to_string()),
            ConnectMode::Ok => {
                let (tx, rx) = mpsc::unbounded_channel();
                let alive = Arc::new(AtomicBool::new(true));
                {
                    let mut p = self.probe.lock().unwrap();
                    p.inject = Some(tx.clone());
                    p.alive = Some(alive.clone());
                }
                Ok(FakeLink { probe: self.probe.clone(), rx, tx, alive, replies: self.replies.clone() })
            }
        }
    }
}

fn m60(id: &str, rssi: i16) -> Advert {
    Advert {
        id: id.into(),
        services: vec![p::ADV_SERVICE],
        manufacturer_ids: vec![p::ADV_MANUFACTURER_ID],
        rssi: Some(rssi),
    }
}

fn other(id: &str, rssi: i16) -> Advert {
    Advert { id: id.into(), services: vec![], manufacturer_ids: vec![0x004C], rssi: Some(rssi) }
}

struct Rig {
    handle: Handle,
    probe: Shared,
    task: JoinHandle<()>,
    saved: Arc<Mutex<Vec<String>>>,
}

fn start(adverts: Vec<Advert>, mode: ConnectMode, remembered: Option<&str>) -> Rig {
    start_with(adverts, mode, remembered, false)
}

fn start_shared() -> Rig {
    start_with(vec![m60("AA", -50)], ConnectMode::Ok, None, true)
}

fn start_with(adverts: Vec<Advert>, mode: ConnectMode, remembered: Option<&str>, share: bool) -> Rig {
    let probe: Shared = Arc::default();
    let radio = FakeRadio { probe: probe.clone(), adverts, mode, replies: default_replies() };
    let saved = Arc::new(Mutex::new(Vec::new()));
    let s2 = saved.clone();
    let settings = Settings {
        remembered: remembered.map(str::to_string),
        on_connected: Box::new(move |id| s2.lock().unwrap().push(id.to_string())),
        share,
        ..Settings::default()
    };
    let (ctrl, handle) = Controller::new(radio, settings);
    Rig { handle, probe, task: tokio::spawn(ctrl.run()), saved }
}

async fn until(rig: &Rig, what: &str, cond: impl Fn(&State) -> bool) -> State {
    let mut rx = rig.handle.subscribe();
    let found = match timeout(Duration::from_secs(300), rx.wait_for(|s| cond(s))).await {
        Ok(Ok(s)) => Some(s.clone()),
        _ => None,
    };
    found.unwrap_or_else(|| panic!("timed out waiting for: {what}; state = {:?}", rig.handle.state()))
}

/// Connected and finished the initial reads (firmware and model are the last two replies).
async fn ready(rig: &Rig) -> State {
    until(rig, "initial reads", |s| s.connected && s.model.is_some() && s.firmware.is_some()).await
}

fn written(rig: &Rig) -> Vec<Vec<u8>> {
    rig.probe.lock().unwrap().written.clone()
}

#[tokio::test(start_paused = true)]
async fn connects_reads_everything_in_order_and_fills_state() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    let s = ready(&rig).await;
    let want: Vec<Vec<u8>> = INITIAL_READS.iter().map(|c| build_frame(*c, &[])).collect();
    assert_eq!(written(&rig), want);
    assert_eq!(
        (s.volume, s.input, s.eq_preset, s.backlight_duration, s.backlight_sensitivity, s.bt_prompt),
        (Some(7), Some(1), Some(0), Some(1), Some(1), Some(true))
    );
    assert_eq!(s.firmware.as_deref(), Some("2.4.1"));
    assert_eq!(s.model.as_deref(), Some("EDIFIER M60"));
    assert_eq!(s.status_line(), "Connected - volume 7 - Bluetooth - EQ Music");
    assert_eq!(rig.saved.lock().unwrap().as_slice(), ["AA"]);
}

#[tokio::test(start_paused = true)]
async fn picks_the_strongest_matching_device_and_ignores_others() {
    let adverts = vec![
        other("LOUD", -30),
        m60("WEAK", -80),
        m60("STRONG", -45),
        Advert { id: "HALF".into(), services: vec![p::ADV_SERVICE], manufacturer_ids: vec![], rssi: Some(-20) },
    ];
    let rig = start(adverts, ConnectMode::Ok, None);
    ready(&rig).await;
    assert_eq!(rig.probe.lock().unwrap().connects, ["STRONG"], "full match beats a stronger half match");
}

#[tokio::test(start_paused = true)]
async fn remembered_device_is_preferred_when_present() {
    let rig = start(vec![m60("NEW", -40), m60("OLD", -90)], ConnectMode::Ok, Some("OLD"));
    ready(&rig).await;
    assert_eq!(rig.probe.lock().unwrap().connects, ["OLD"]);
    assert_eq!(rig.probe.lock().unwrap().scanned_prefer[0].as_deref(), Some("OLD"));
}

#[tokio::test(start_paused = true)]
async fn remembered_device_missing_falls_back_to_scan_result() {
    let rig = start(vec![m60("NEW", -40)], ConnectMode::Ok, Some("GONE"));
    ready(&rig).await;
    assert_eq!(rig.probe.lock().unwrap().connects, ["NEW"]);
}

#[tokio::test(start_paused = true)]
async fn not_found_backs_off_2_4_8_16_then_20_seconds() {
    let rig = start(vec![other("X", -40)], ConnectMode::Ok, None);
    let s = until(&rig, "not found", |s| s.status.starts_with("Speaker not found")).await;
    assert!(s.status.contains("in use by another app or user"), "{}", s.status);
    tokio::time::sleep(Duration::from_secs(120)).await;
    let scans = rig.probe.lock().unwrap().scans.clone();
    let gaps: Vec<u64> = scans.windows(2).map(|w| (w[1] - w[0]).as_secs()).collect();
    assert_eq!(&gaps[..6], &[2, 4, 8, 16, 20, 20]);
}

#[tokio::test(start_paused = true)]
async fn a_hanging_connect_times_out_instead_of_sitting_on_connecting() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Hang, None);
    until(&rig, "connecting", |s| s.status == "Connecting...").await;
    let s = until(&rig, "timeout", |s| s.status.starts_with("Connection failed")).await;
    assert_eq!(s.status, "Connection failed: timed out");
    assert!(rig.saved.lock().unwrap().is_empty(), "nothing remembered for a failed connect");
}

#[tokio::test(start_paused = true)]
async fn connect_error_is_reported_and_retried() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Fail("busy"), None);
    until(&rig, "failure", |s| s.status == "Connection failed: busy").await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(rig.probe.lock().unwrap().connects.len() >= 3);
}

#[tokio::test(start_paused = true)]
async fn link_loss_clears_state_and_reconnects() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    ready(&rig).await;
    let tx = rig.probe.lock().unwrap().inject.clone().unwrap();
    tx.send(Msg::Closed).unwrap();
    let s = until(&rig, "disconnect", |s| !s.connected).await;
    assert_eq!((s.volume, s.firmware.clone(), s.model.clone()), (None, None, None));
    assert_eq!(s.status, "Disconnected - retrying...");
    ready(&rig).await; // and it comes back
    assert_eq!(rig.probe.lock().unwrap().connects.len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_silently_dead_link_is_noticed_by_polling() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    ready(&rig).await;
    rig.probe.lock().unwrap().alive.clone().unwrap().store(false, Ordering::SeqCst);
    until(&rig, "poll notices", |s| !s.connected).await;
}

#[tokio::test(start_paused = true)]
async fn commands_produce_the_bytes_the_official_app_sent() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    ready(&rig).await;
    let base = written(&rig).len();
    let h = &rig.handle;
    h.send(Command::SetVolume(7));
    h.send(Command::SetVolume(99)); // clamps to 16
    h.send(Command::StepVolume(-1)); // 16 -> 15
    h.send(Command::SetInput(2));
    h.send(Command::SetInput(3));
    h.send(Command::SetEq(4));
    h.send(Command::SetBacklightDuration(0)); // sensitivity stays General
    h.send(Command::SetBacklightSensitivity(0)); // duration stays 5 s
    h.send(Command::SetBtPrompt(false));
    h.send(Command::SetBtPrompt(true));
    h.send(Command::PowerOff);
    // power off is last, so once the speaker is off everything before it has been sent
    until(&rig, "all sent", |s| s.status == "Speaker turned off").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let hex = |b: &Vec<u8>| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let got: Vec<String> = written(&rig)[base..].iter().map(hex).collect();
    assert_eq!(
        got,
        [
            "aaec6700010705",     // volume 7 (captured)
            "aaec670001100e",     // volume 16 (captured)
            "aaec6700010f0d",     // volume 15 (captured)
            "aaec6200020f020b",   // USB (captured)
            "aaec6200020f030c",   // AUX (captured)
            "aaecc40001045f",     // EQ custom (captured)
            "aaec85000303000122", // 5 s + General (captured)
            "aaec85000303000021", // 5 s + High
            "aaec87000302010023", // prompt off (captured)
            "aaec87000302010124", // prompt on (captured)
            "aaecce000064",       // power off (captured)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn invalid_values_and_unknown_dependencies_send_nothing() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    ready(&rig).await;
    let base = written(&rig).len();
    let h = &rig.handle;
    h.send(Command::SetInput(9));
    h.send(Command::SetEq(9));
    h.send(Command::SetBacklightDuration(7));
    h.send(Command::SetBacklightSensitivity(9));
    h.send(Command::Media(0x02)); // not an action the speaker documents
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(written(&rig).len(), base);
}

#[tokio::test(start_paused = true)]
async fn backlight_with_unknown_other_field_sends_nothing() {
    let mut replies = default_replies();
    replies.remove(&p::CMD_BACKLIGHT_GET); // the speaker never answers that read
    let probe: Shared = Arc::default();
    let radio = FakeRadio { probe: probe.clone(), adverts: vec![m60("AA", -50)], mode: ConnectMode::Ok, replies };
    let (ctrl, handle) = Controller::new(radio, Settings::default());
    tokio::spawn(ctrl.run());
    let mut rx = handle.subscribe();
    rx.wait_for(|s| s.model.is_some()).await.unwrap();
    let base = probe.lock().unwrap().written.len();
    handle.send(Command::SetBacklightDuration(2));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(probe.lock().unwrap().written.len(), base);
}

#[tokio::test(start_paused = true)]
async fn media_is_only_sent_on_the_bluetooth_input() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    ready(&rig).await; // input is Bluetooth
    let base = written(&rig).len();
    rig.handle.send(Command::Media(4));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(written(&rig)[base], build_frame(p::CMD_MEDIA, &[4]));
    assert_eq!(written(&rig)[base], hex_bytes("aaecc20001045d"));

    rig.handle.send(Command::SetInput(3));
    until(&rig, "aux", |s| s.input == Some(3)).await;
    let n = written(&rig).len();
    rig.handle.send(Command::Media(4));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(written(&rig).len(), n, "no media on AUX");
}

fn hex_bytes(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// Captured `0x43` reply payload: the speaker holds the "Gaming" profile.
const GAMING_43: &str =
    "03060000003e0007010000fa0007020003e8060703000fa0060704001f40060705003e8006073afbf26a47616d696e67";

#[tokio::test(start_paused = true)]
async fn reads_the_custom_profile_on_connect() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    let s = ready(&rig).await;
    let eq = s.custom_eq.expect("custom profile read");
    assert_eq!((eq.name.as_str(), eq.id()), ("Gaming", 1_794_308_922));
}

#[tokio::test(start_paused = true)]
async fn applying_a_custom_preset_selects_custom_then_writes_the_profile_like_the_app() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    ready(&rig).await; // EQ preset is Music (0)
    let base = written(&rig).len();
    let music = CustomEq::new("Music", [0.5, 1.0, -1.0, 0.5, 0.0, -1.0], 0x6af2_fa92);
    rig.handle.send(Command::ApplyCustomEq(music.clone()));
    let s = until(&rig, "applied", |s| s.custom_eq.as_ref().is_some_and(|c| c.name == "Music")).await;
    assert_eq!(s.eq_preset, Some(p::EQ_CUSTOM));
    let hex = |b: &Vec<u8>| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let got: Vec<String> = written(&rig)[base..].iter().map(hex).collect();
    assert_eq!(
        got,
        [
            "aaecc40001045f", // EQ Custom (captured)
            "aaec46002d0000003e0707010000fa0807020003e8040703000fa0070704001f40060705003e80040792faf26a4d757369633e", // captured
        ]
    );

    // already on Custom: switching profile is just the 46 write (as in the capture)
    let base = written(&rig).len();
    let gaming = CustomEq::new("Gaming", [-3.0, -3.0, 0.0, 0.0, 0.0, 0.0], 0x6af2_fb3a);
    rig.handle.send(Command::ApplyCustomEq(gaming));
    until(&rig, "gaming", |s| s.custom_eq.as_ref().is_some_and(|c| c.name == "Gaming")).await;
    let got: Vec<String> = written(&rig)[base..].iter().map(hex).collect();
    assert_eq!(
        got,
        ["aaec46002e0000003e0007010000fa0007020003e8060703000fa0060704001f40060705003e8006073afbf26a47616d696e672e"]
    );
}

#[tokio::test(start_paused = true)]
async fn an_unnamed_or_malformed_profile_is_not_sent() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    ready(&rig).await;
    let base = written(&rig).len();
    rig.handle.send(Command::ApplyCustomEq(CustomEq::new("   ", [0.0; 6], 1)));
    let mut short = CustomEq::new("x", [0.0; 6], 1);
    short.bands.pop();
    rig.handle.send(Command::ApplyCustomEq(short));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(written(&rig).len(), base);
}

#[tokio::test(start_paused = true)]
async fn speaker_pushes_update_state() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    ready(&rig).await;
    let tx = rig.probe.lock().unwrap().inject.clone().unwrap();
    // the speaker pushes these on its own (e.g. a hardware knob or the phone app)
    tx.send(Msg::Data(rsp(0x66, &[0x10, 12]))).unwrap();
    tx.send(Msg::Data(rsp(0x61, &[0x0F, 3]))).unwrap();
    let s = until(&rig, "push", |s| s.volume == Some(12) && s.input == Some(3)).await;
    assert_eq!(s.status_line(), "Connected - volume 12 - AUX - EQ Music");
}

#[tokio::test(start_paused = true)]
async fn notifications_split_across_chunks_still_apply() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    ready(&rig).await;
    let tx = rig.probe.lock().unwrap().inject.clone().unwrap();
    let f = rsp(0x66, &[0x10, 3]);
    tx.send(Msg::Data(f[..3].to_vec())).unwrap();
    tx.send(Msg::Data(f[3..].to_vec())).unwrap();
    until(&rig, "split", |s| s.volume == Some(3)).await;
}

#[tokio::test(start_paused = true)]
async fn commands_sent_while_disconnected_are_not_replayed_later() {
    let rig = start(vec![other("X", -50)], ConnectMode::Ok, None);
    until(&rig, "searching", |s| s.status.starts_with("Speaker not found")).await;
    rig.handle.send(Command::SetVolume(3)); // dropped: there is no speaker
    rig.handle.send(Command::PowerOff);
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(rig.probe.lock().unwrap().written.is_empty());
}

#[tokio::test(start_paused = true)]
async fn reconnect_command_drops_the_link_and_searches_again() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    ready(&rig).await;
    rig.handle.send(Command::Reconnect);
    // reconnecting is instant with the fake radio, so the "disconnected" state is never observable
    tokio::time::sleep(Duration::from_secs(5)).await;
    ready(&rig).await;
    let p = rig.probe.lock().unwrap();
    assert_eq!((p.connects.len(), p.closed), (2, 1));
}

#[tokio::test(start_paused = true)]
async fn quit_closes_the_link_and_ends_the_task() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    ready(&rig).await;
    rig.handle.send(Command::Quit);
    timeout(Duration::from_secs(5), rig.task).await.expect("task ends").unwrap();
    assert_eq!(rig.handle.state().status, "Stopped");
    assert_eq!(rig.probe.lock().unwrap().closed, 1);
}

#[tokio::test(start_paused = true)]
async fn quit_while_searching_ends_the_task() {
    let rig = start(vec![], ConnectMode::Ok, None);
    until(&rig, "not found", |s| s.status.starts_with("Speaker not found")).await;
    rig.handle.send(Command::Quit);
    timeout(Duration::from_secs(5), rig.task).await.expect("task ends").unwrap();
}

#[tokio::test(start_paused = true)]
async fn dropping_every_handle_stops_the_controller() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    ready(&rig).await;
    let Rig { handle, task, .. } = rig;
    drop(handle);
    timeout(Duration::from_secs(5), task).await.expect("task ends").unwrap();
}

// ---- sharing mode --------------------------------------------------------------------------------

fn counts(rig: &Rig) -> (usize, usize) {
    let p = rig.probe.lock().unwrap();
    (p.connects.len(), p.closed)
}

/// Started in sharing mode: connected once, read everything, and let go again.
async fn released(rig: &Rig) -> State {
    until(rig, "released with settings", |s| s.idle && !s.connected && s.model.is_some()).await
}

fn hexes(frames: &[Vec<u8>]) -> Vec<String> {
    frames.iter().map(|b| b.iter().map(|x| format!("{x:02x}")).collect()).collect()
}

#[tokio::test(start_paused = true)]
async fn sharing_reads_the_speaker_then_lets_go_keeping_the_settings() {
    let rig = start_shared();
    let s = released(&rig).await;
    assert_eq!(counts(&rig), (1, 1), "one connection, closed again");
    assert_eq!((s.volume, s.input, s.eq_preset), (Some(7), Some(1), Some(0)), "last known settings kept");
    assert!(s.controllable(), "the UI can still offer the controls");
    assert!(s.sharing);
    assert_eq!(s.status_line(), "Speaker free for other devices - volume 7 - Bluetooth - EQ Music");
    let want: Vec<Vec<u8>> = INITIAL_READS.iter().map(|c| build_frame(*c, &[])).collect();
    assert_eq!(written(&rig), want, "only reads were sent");
}

#[tokio::test(start_paused = true)]
async fn a_change_while_released_connects_sends_it_first_and_lets_go_after_20_s() {
    let rig = start_shared();
    released(&rig).await;
    let base = written(&rig).len();
    rig.handle.send(Command::SetVolume(9));
    until(&rig, "volume 9", |s| s.volume == Some(9) && s.connected).await;
    assert_eq!(counts(&rig).0, 2);
    assert_eq!(hexes(&written(&rig)[base..base + 1]), ["aaec6700010907"], "the change goes out before the reads");
    tokio::time::sleep(Duration::from_secs(15)).await;
    assert_eq!(counts(&rig), (2, 1), "still held 15 s later");
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(counts(&rig), (2, 2), "let go after 20 s without use");
    let s = rig.handle.state();
    assert!(s.idle && s.volume == Some(9) && s.status == "Speaker free for other devices");
}

#[tokio::test(start_paused = true)]
async fn each_use_extends_the_hold() {
    let rig = start_shared();
    released(&rig).await;
    rig.handle.send(Command::SetInput(3));
    until(&rig, "connected", |s| s.connected && s.input == Some(3)).await;
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_secs(15)).await;
        rig.handle.send(Command::SetEq(2));
    }
    assert_eq!(counts(&rig), (2, 1), "kept for a minute of steady use");
    tokio::time::sleep(Duration::from_secs(25)).await;
    assert_eq!(counts(&rig), (2, 2));
}

#[tokio::test(start_paused = true)]
async fn a_slider_drag_while_connecting_sends_only_where_it_ended() {
    let rig = start_shared();
    released(&rig).await;
    let base = written(&rig).len();
    for v in [3, 4, 5, 6] {
        rig.handle.send(Command::SetVolume(v));
    }
    until(&rig, "volume 6", |s| s.volume == Some(6) && s.connected).await;
    let sent: Vec<String> = hexes(&written(&rig)[base..]).into_iter().filter(|h| h.starts_with("aaec67")).collect();
    assert_eq!(sent, ["aaec6700010604"], "{sent:?}");
}

#[tokio::test(start_paused = true)]
async fn the_link_is_held_while_the_ui_is_busy() {
    let rig = start_shared();
    released(&rig).await;
    rig.handle.send(Command::SetBusy(true));
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(counts(&rig), (2, 1), "busy connects at once");
    tokio::time::sleep(Duration::from_secs(300)).await;
    assert_eq!(counts(&rig), (2, 1), "and holds for as long as it is busy");
    rig.handle.send(Command::SetBusy(false));
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(counts(&rig), (2, 1), "a short grace after closing the popup");
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(counts(&rig), (2, 2), "then lets go");
}

#[tokio::test(start_paused = true)]
async fn wake_connects_ahead_of_use_without_sending_changes() {
    let rig = start_shared();
    released(&rig).await;
    let base = written(&rig).len();
    rig.handle.send(Command::Wake);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(counts(&rig), (2, 1));
    assert!(written(&rig)[base..].iter().all(|f| f[3..5] == [0, 0]), "only reads");
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(counts(&rig), (2, 2));
}

#[tokio::test(start_paused = true)]
async fn while_released_it_polls_once_a_minute() {
    let rig = start_shared();
    released(&rig).await;
    tokio::time::sleep(Duration::from_secs(200)).await;
    let (connects, closed) = counts(&rig);
    assert_eq!((connects, closed), (4, 4), "start-up plus three polls, each let go again");
}

#[tokio::test(start_paused = true)]
async fn a_poll_lets_go_quickly() {
    let rig = start_shared();
    released(&rig).await;
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert_eq!(counts(&rig).0, 2, "polled");
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(counts(&rig), (2, 2), "reads, a moment for the replies, then released");
}

#[tokio::test(start_paused = true)]
async fn turning_sharing_off_connects_and_stays_and_on_again_lets_go() {
    let rig = start_shared();
    released(&rig).await;
    rig.handle.send(Command::SetSharing(false));
    tokio::time::sleep(Duration::from_secs(300)).await;
    assert_eq!(counts(&rig), (2, 1), "connected and held");
    let s = rig.handle.state();
    assert!(s.connected && !s.sharing && !s.idle);
    rig.handle.send(Command::SetSharing(true));
    tokio::time::sleep(Duration::from_secs(25)).await;
    assert_eq!(counts(&rig), (2, 2), "released again");
    assert!(rig.handle.state().sharing);
}

#[tokio::test(start_paused = true)]
async fn turning_sharing_on_while_connected_lets_go_after_the_hold() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    ready(&rig).await;
    rig.handle.send(Command::SetSharing(true));
    tokio::time::sleep(Duration::from_secs(15)).await;
    assert_eq!(counts(&rig), (1, 0));
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(counts(&rig), (1, 1));
    assert!(rig.handle.state().idle);
}

#[tokio::test(start_paused = true)]
async fn an_unreachable_speaker_keeps_the_last_known_settings_and_drops_the_change() {
    let rig = start_shared();
    released(&rig).await;
    let base = written(&rig).len();
    rig.probe.lock().unwrap().offline = true;
    rig.handle.send(Command::SetVolume(2));
    let s = until(&rig, "unavailable", |s| s.status.starts_with("Speaker unavailable")).await;
    assert_eq!((s.volume, s.idle, s.connected), (Some(7), true, false), "last known settings stay");
    assert_eq!(
        s.status_line(),
        "Speaker unavailable (off, or in use by another device) - volume 7 - Bluetooth - EQ Music"
    );
    assert_eq!(written(&rig).len(), base, "nothing sent");
    // it comes back at the next poll, and the dropped change is not sent then
    rig.probe.lock().unwrap().offline = false;
    tokio::time::sleep(Duration::from_secs(65)).await;
    assert_eq!(counts(&rig).0, 2);
    assert!(hexes(&written(&rig)[base..]).iter().all(|h| !h.starts_with("aaec67")));
    assert_eq!(rig.handle.state().status, "Speaker free for other devices");
}

#[tokio::test(start_paused = true)]
async fn losing_the_link_while_sharing_keeps_the_settings() {
    let rig = start_shared();
    released(&rig).await;
    rig.handle.send(Command::Wake);
    tokio::time::sleep(Duration::from_secs(2)).await;
    rig.probe.lock().unwrap().inject.clone().unwrap().send(Msg::Closed).unwrap();
    let s = until(&rig, "lost", |s| s.status.starts_with("Disconnected")).await;
    assert_eq!((s.volume, s.idle), (Some(7), true));
}

#[tokio::test(start_paused = true)]
async fn nothing_known_yet_means_no_controls_while_released() {
    let rig = start_with(vec![], ConnectMode::Ok, None, true);
    let s = until(&rig, "not found", |s| s.status.starts_with("Speaker not found")).await;
    assert!(!s.controllable(), "no last known settings to offer");
    // a click (or the Reconnect item) still tries again
    rig.handle.send(Command::Reconnect);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(rig.probe.lock().unwrap().scans.len(), 2);
}

#[tokio::test(start_paused = true)]
async fn quit_while_released_ends_the_task() {
    let rig = start_shared();
    released(&rig).await;
    rig.handle.send(Command::Quit);
    timeout(Duration::from_secs(5), rig.task).await.expect("task ends").unwrap();
}

// ---- volume per input ---------------------------------------------------------------------------

type Levels = Arc<Mutex<HashMap<u8, u8>>>;

/// Volume memory on, with `levels` remembered from before. `on_aux` makes the speaker report AUX
/// when we connect (it is on Bluetooth otherwise).
fn start_vm(levels: &[(u8, u8)], share: bool, on_aux: Option<u8>) -> (Rig, Levels) {
    let probe: Shared = Arc::default();
    if let Some(vol) = on_aux {
        let mut p = probe.lock().unwrap();
        p.remembered.insert(0x61, rsp(0x61, &[0x0F, 3]));
        p.remembered.insert(0x66, rsp(0x66, &[0x10, vol]));
    }
    let radio = FakeRadio {
        probe: probe.clone(),
        adverts: vec![m60("AA", -50)],
        mode: ConnectMode::Ok,
        replies: default_replies(),
    };
    let saved: Levels = Arc::default();
    let s2 = saved.clone();
    let settings = Settings {
        share,
        remember_volume: true,
        input_volumes: levels.iter().copied().collect(),
        on_input_volumes: Box::new(move |l| *s2.lock().unwrap() = l.clone()),
        ..Settings::default()
    };
    let (ctrl, handle) = Controller::new(radio, settings);
    (Rig { handle, probe, task: tokio::spawn(ctrl.run()), saved: Arc::default() }, saved)
}

fn push(rig: &Rig, cmd: u8, payload: &[u8]) {
    rig.probe.lock().unwrap().inject.clone().unwrap().send(Msg::Data(rsp(cmd, payload))).unwrap();
}

/// Volume frames written since `base`.
fn volume_writes(rig: &Rig, base: usize) -> Vec<u8> {
    written(rig)[base..].iter().filter(|f| f[2] == p::CMD_SET_VOLUME).map(|f| f[5]).collect()
}

async fn settle() {
    tokio::time::sleep(Duration::from_secs(3)).await;
}

#[tokio::test(start_paused = true)]
async fn switching_back_to_aux_restores_its_level() {
    let (rig, saved) = start_vm(&[], false, None);
    ready(&rig).await; // Bluetooth, volume 7
    settle().await;
    let base = written(&rig).len();
    rig.handle.send(Command::SetInput(3));
    settle().await;
    assert!(volume_writes(&rig, base).is_empty(), "nothing remembered for AUX yet");
    assert_eq!(saved.lock().unwrap().get(&3), Some(&7), "AUX starts with the level it had");
    rig.handle.send(Command::SetVolume(4));
    until(&rig, "4", |s| s.volume == Some(4)).await;
    assert_eq!(saved.lock().unwrap().get(&3), Some(&4));

    // the speaker goes to Bluetooth and the phone sets its own level
    push(&rig, 0x61, &[0x0F, 1]);
    tokio::time::sleep(Duration::from_millis(300)).await;
    push(&rig, 0x66, &[0x10, 12]);
    settle().await;
    assert_eq!(rig.handle.state().volume, Some(12));
    assert_eq!(*saved.lock().unwrap(), HashMap::from([(3, 4)]), "Bluetooth levels are not remembered");

    // back to AUX from the app: its level comes back
    let base = written(&rig).len();
    rig.handle.send(Command::SetInput(3));
    until(&rig, "restored", |s| s.input == Some(3) && s.volume == Some(4)).await;
    let got = hexes(&written(&rig)[base..]);
    assert_eq!(
        got,
        vec!["aaec6200020f030c".to_string(), hexes(&[build_frame(0x67, &[4])])[0].clone()],
        "switch, then its level"
    );
}

#[tokio::test(start_paused = true)]
async fn a_switch_on_the_speaker_itself_restores_too() {
    let (rig, _) = start_vm(&[(3, 5)], false, None);
    ready(&rig).await;
    settle().await;
    let base = written(&rig).len();
    push(&rig, 0x61, &[0x0F, 3]); // input button / remote
    until(&rig, "restored", |s| s.volume == Some(5)).await;
    assert_eq!(volume_writes(&rig, base), [5]);
}

#[tokio::test(start_paused = true)]
async fn an_override_right_after_the_switch_is_put_back_once() {
    let (rig, _) = start_vm(&[(3, 5)], false, None);
    ready(&rig).await;
    settle().await;
    let base = written(&rig).len();
    push(&rig, 0x61, &[0x0F, 3]);
    tokio::time::sleep(Duration::from_millis(500)).await;
    push(&rig, 0x66, &[0x10, 11]); // something sets its own level just after the switch
    tokio::time::sleep(Duration::from_millis(500)).await;
    push(&rig, 0x66, &[0x10, 12]); // and again
    settle().await;
    assert_eq!(volume_writes(&rig, base), [5, 5], "restored, and put back once, no tug of war");
}

#[tokio::test(start_paused = true)]
async fn bluetooth_is_left_to_the_connected_device() {
    let (rig, saved) = start_vm(&[(1, 3), (3, 5)], false, None);
    ready(&rig).await;
    settle().await;
    push(&rig, 0x61, &[0x0F, 3]);
    settle().await;
    let base = written(&rig).len();
    push(&rig, 0x61, &[0x0F, 1]);
    settle().await;
    assert!(volume_writes(&rig, base).is_empty(), "no level forced on Bluetooth");
    assert!(!saved.lock().unwrap().contains_key(&1) || saved.lock().unwrap()[&1] == 3);
}

#[tokio::test(start_paused = true)]
async fn finding_another_input_on_connect_does_not_change_the_volume() {
    let (rig, saved) = start_vm(&[(3, 5)], false, Some(9)); // the speaker is on AUX at 9
    let s = ready(&rig).await;
    settle().await;
    assert_eq!((s.input, s.volume), (Some(3), Some(9)));
    assert!(written(&rig).iter().all(|f| f[2] != p::CMD_SET_VOLUME), "no jump on connect");
    assert_eq!(saved.lock().unwrap().get(&3), Some(&9), "the level found becomes the remembered one");
}

#[tokio::test(start_paused = true)]
async fn switched_off_it_does_nothing() {
    let (rig, saved) = start_vm(&[(3, 5)], false, None);
    ready(&rig).await;
    rig.handle.send(Command::SetVolumeMemory(false));
    until(&rig, "off", |s| !s.volume_memory).await;
    let base = written(&rig).len();
    push(&rig, 0x61, &[0x0F, 3]);
    settle().await;
    rig.handle.send(Command::SetVolume(2));
    settle().await;
    assert_eq!(volume_writes(&rig, base), [2], "only the user's own change");
    assert_eq!(*saved.lock().unwrap(), HashMap::new(), "nothing recorded while off");
}

#[tokio::test(start_paused = true)]
async fn sharing_a_switch_from_the_app_restores_after_connecting() {
    let (rig, _) = start_vm(&[(3, 5)], true, None);
    released(&rig).await; // last known: Bluetooth, 7
    let base = written(&rig).len();
    rig.handle.send(Command::SetInput(3));
    until(&rig, "restored", |s| s.input == Some(3) && s.volume == Some(5)).await;
    let got = hexes(&written(&rig)[base..base + 2]);
    assert_eq!(got[0], "aaec6200020f030c");
    assert_eq!(got[1], hexes(&[build_frame(0x67, &[5])])[0]);
}

#[tokio::test(start_paused = true)]
async fn sharing_a_poll_that_finds_another_input_does_not_change_the_volume() {
    let (rig, _) = start_vm(&[(3, 5)], true, None);
    released(&rig).await; // Bluetooth, 7
    {
        // meanwhile the phone switched the speaker to AUX at 9
        let mut p = rig.probe.lock().unwrap();
        p.remembered.insert(0x61, rsp(0x61, &[0x0F, 3]));
        p.remembered.insert(0x66, rsp(0x66, &[0x10, 9]));
    }
    let base = written(&rig).len();
    tokio::time::sleep(Duration::from_secs(65)).await; // a poll
    let s = rig.handle.state();
    assert_eq!((s.input, s.volume), (Some(3), Some(9)));
    assert!(volume_writes(&rig, base).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_change_by_the_user_right_after_a_switch_is_kept_and_remembered() {
    let (rig, saved) = start_vm(&[(3, 5)], false, None);
    ready(&rig).await;
    settle().await;
    let base = written(&rig).len();
    rig.handle.send(Command::SetInput(3));
    tokio::time::sleep(Duration::from_millis(300)).await;
    rig.handle.send(Command::SetVolume(8)); // the user moves the slider straight away
    settle().await;
    assert_eq!(volume_writes(&rig, base), [5, 8], "restored, then the user's level, not put back");
    assert_eq!(rig.handle.state().volume, Some(8));
    assert_eq!(saved.lock().unwrap().get(&3), Some(&8));
}

#[tokio::test(start_paused = true)]
async fn connecting_is_flagged_while_it_lasts() {
    let rig = start(vec![m60("AA", -50)], ConnectMode::Hang, None);
    until(&rig, "connecting", |s| s.connecting && s.status == "Connecting...").await;
    let s = until(&rig, "gave up", |s| s.status.starts_with("Connection failed")).await;
    assert!(!s.connecting, "cleared when the attempt ends");

    let rig = start(vec![m60("AA", -50)], ConnectMode::Ok, None);
    let s = ready(&rig).await;
    assert!(!s.connecting, "cleared once connected");
}

// ---- power off into Bluetooth ---------------------------------------------------------------------

type Resumes = Arc<Mutex<Vec<Option<Resume>>>>;

/// Power off into Bluetooth on; the speaker starts on `input` at `volume`.
fn start_power(input: u8, volume: u8, share: bool, resume: Option<Resume>, enabled: bool) -> (Rig, Resumes) {
    let probe: Shared = Arc::default();
    {
        let mut p = probe.lock().unwrap();
        p.remembered.insert(0x61, rsp(0x61, &[0x0F, input]));
        p.remembered.insert(0x66, rsp(0x66, &[0x10, volume]));
    }
    let radio = FakeRadio {
        probe: probe.clone(),
        adverts: vec![m60("AA", -50)],
        mode: ConnectMode::Ok,
        replies: default_replies(),
    };
    let saved: Resumes = Arc::default();
    let s2 = saved.clone();
    let settings = Settings {
        share,
        bluetooth_on_power_off: enabled,
        resume,
        on_resume: Box::new(move |r| s2.lock().unwrap().push(r)),
        ..Settings::default()
    };
    let (ctrl, handle) = Controller::new(radio, settings);
    (Rig { handle, probe, task: tokio::spawn(ctrl.run()), saved: Arc::default() }, saved)
}

fn written_hex(rig: &Rig, base: usize) -> Vec<String> {
    hexes(&written(rig)[base..])
}

/// The speaker is switched on again (it starts on `input`).
fn switch_on(rig: &Rig, input: u8, volume: u8) {
    let mut p = rig.probe.lock().unwrap();
    p.remembered.insert(0x61, rsp(0x61, &[0x0F, input]));
    p.remembered.insert(0x66, rsp(0x66, &[0x10, volume]));
    p.offline = false;
}

const TO_BLUETOOTH: &str = "aaec6200020f010a";
const TO_AUX: &str = "aaec6200020f030c";
const POWER_OFF: &str = "aaecce000064";

#[tokio::test(start_paused = true)]
async fn power_off_from_aux_switches_to_bluetooth_first_and_remembers() {
    let (rig, saved) = start_power(3, 9, false, None, true);
    ready(&rig).await;
    let base = written(&rig).len();
    rig.handle.send(Command::PowerOff);
    let s = until(&rig, "off", |s| s.status == "Speaker turned off").await;
    assert_eq!(written_hex(&rig, base), [TO_BLUETOOTH, POWER_OFF], "Bluetooth first, then off");
    assert_eq!(*saved.lock().unwrap(), [Some(Resume { input: 3, volume: Some(9) })], "remembered (and saved)");
    assert_eq!(s.resume, Some(Resume { input: 3, volume: Some(9) }));
    assert!(!s.controllable(), "nothing to control while it is off");
}

#[tokio::test(start_paused = true)]
async fn when_it_comes_back_on_bluetooth_the_input_and_volume_are_put_back() {
    let (rig, saved) = start_power(3, 9, false, None, true);
    ready(&rig).await;
    rig.handle.send(Command::PowerOff);
    until(&rig, "off", |s| s.status == "Speaker turned off").await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    switch_on(&rig, 1, 14); // it starts on Bluetooth, where the phone sets 14
    let base = written(&rig).len();
    let s = until(&rig, "restored", |s| s.connected && s.input == Some(3) && s.volume == Some(9)).await;
    let after_reads: Vec<String> =
        written_hex(&rig, base).into_iter().filter(|h| !h.ends_with("0000fc") && h[8..10] != *"00").collect();
    assert_eq!(after_reads, [TO_AUX.to_string(), hexes(&[build_frame(0x67, &[9])])[0].clone()], "AUX, then its volume");
    assert_eq!(s.resume, None);
    assert_eq!(saved.lock().unwrap().last(), Some(&None), "cleared once done");
}

#[tokio::test(start_paused = true)]
async fn waiting_for_it_to_come_back_looks_every_10_seconds() {
    let (rig, _) = start_power(3, 9, false, None, true);
    ready(&rig).await;
    rig.handle.send(Command::PowerOff);
    until(&rig, "off", |s| s.status == "Speaker turned off").await;
    let scans = rig.probe.lock().unwrap().scans.len();
    tokio::time::sleep(Duration::from_secs(120)).await;
    let more = rig.probe.lock().unwrap().scans.len() - scans;
    assert!((10..=14).contains(&more), "about every 10 s, not backing off to 20 s ({more} scans in 2 min)");
    assert_eq!(rig.handle.state().status, "Speaker turned off");
}

#[tokio::test(start_paused = true)]
async fn if_someone_picked_another_input_meanwhile_it_is_left_alone() {
    let (rig, saved) = start_power(3, 9, false, None, true);
    ready(&rig).await;
    rig.handle.send(Command::PowerOff);
    until(&rig, "off", |s| s.status == "Speaker turned off").await;
    switch_on(&rig, 2, 5); // on USB now
    let base = written(&rig).len();
    until(&rig, "back", |s| s.connected && s.model.is_some()).await;
    settle().await;
    let s = rig.handle.state();
    assert!(written_hex(&rig, base).iter().all(|h| !h.starts_with("aaec62") && !h.starts_with("aaec67")), "no switch");
    assert_eq!((s.input, s.resume), (Some(2), None));
    assert_eq!(saved.lock().unwrap().last(), Some(&None));
}

#[tokio::test(start_paused = true)]
async fn with_the_setting_off_power_off_is_just_power_off() {
    let (rig, saved) = start_power(3, 9, false, None, false);
    ready(&rig).await;
    let base = written(&rig).len();
    rig.handle.send(Command::PowerOff);
    until(&rig, "off", |s| s.status == "Speaker turned off").await;
    assert_eq!(written_hex(&rig, base), [POWER_OFF]);
    assert!(saved.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn already_on_bluetooth_it_just_turns_off() {
    let (rig, saved) = start_power(1, 9, false, None, true);
    ready(&rig).await;
    let base = written(&rig).len();
    rig.handle.send(Command::PowerOff);
    until(&rig, "off", |s| s.status == "Speaker turned off").await;
    assert_eq!(written_hex(&rig, base), [POWER_OFF]);
    assert!(saved.lock().unwrap().is_empty(), "nothing to put back");
}

#[tokio::test(start_paused = true)]
async fn without_a_confirmation_it_waits_3_seconds_then_turns_off_anyway() {
    let (rig, _) = start_power(3, 9, false, None, true);
    rig.probe.lock().unwrap().quiet_inputs = true;
    ready(&rig).await;
    let base = written(&rig).len();
    let t0 = Instant::now();
    rig.handle.send(Command::PowerOff);
    until(&rig, "off", |s| s.status == "Speaker turned off").await;
    assert_eq!(written_hex(&rig, base), [TO_BLUETOOTH, POWER_OFF]);
    assert!(t0.elapsed() >= Duration::from_secs(3), "waited for the confirmation first");
}

#[tokio::test(start_paused = true)]
async fn a_pending_restore_survives_a_restart() {
    // the app was closed while the speaker was off; it is started again and the speaker is on Bluetooth
    let (rig, saved) = start_power(1, 14, false, Some(Resume { input: 3, volume: Some(9) }), true);
    let s = until(&rig, "restored", |s| s.connected && s.input == Some(3) && s.volume == Some(9)).await;
    assert_eq!(s.resume, None);
    assert_eq!(saved.lock().unwrap().last(), Some(&None));
}

#[tokio::test(start_paused = true)]
async fn turning_the_setting_off_forgets_a_pending_restore() {
    let (rig, saved) = start_power(1, 14, false, Some(Resume { input: 3, volume: Some(9) }), true);
    rig.probe.lock().unwrap().offline = true; // still off
    tokio::time::sleep(Duration::from_secs(1)).await;
    rig.handle.send(Command::SetBluetoothOnPowerOff(false));
    until(&rig, "forgotten", |s| s.resume.is_none() && !s.bluetooth_on_power_off).await;
    assert_eq!(saved.lock().unwrap().last(), Some(&None));
    rig.probe.lock().unwrap().offline = false;
    let s = until(&rig, "back", |s| s.connected && s.model.is_some()).await;
    settle().await;
    assert_eq!(s.input, Some(1), "stays on Bluetooth");
}

#[tokio::test(start_paused = true)]
async fn sharing_the_restore_happens_on_the_next_look_and_then_lets_go() {
    let (rig, _) = start_power(3, 9, true, None, true);
    released(&rig).await; // AUX, 9
    rig.handle.send(Command::PowerOff); // connects, switches, turns off
    until(&rig, "off", |s| s.status == "Speaker turned off").await;
    switch_on(&rig, 1, 14);
    tokio::time::sleep(Duration::from_secs(12)).await; // a look every 10 s while waiting
    let s = rig.handle.state();
    assert_eq!((s.input, s.volume, s.resume), (Some(3), Some(9), None), "put back");
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(!rig.handle.state().connected, "and released again");
}
