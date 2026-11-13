//! Manages the BLE connection to the speaker and turns [`Command`]s into protocol frames.
//!
//! Platform Bluetooth is supplied through [`Radio`] / [`Link`]. The loop:
//! scan (remembered device first) -> connect (with a time limit) -> read the speaker's
//! settings -> apply commands and notifications.
//!
//! The speaker accepts one control client at a time, so there are two ways to run:
//! - **Always connected** (default): hold the link; on loss, back off and retry.
//! - **Sharing** ([`Settings::share`] / [`Command::SetSharing`]): connect only when needed and let
//!   go when idle, so the phone app or another computer can use the speaker in between. While
//!   released, the last known settings stay in [`State`]; commands are queued and trigger a
//!   connection; the link is held while the UI says it is busy and for a while after the last
//!   use; and the speaker is polled every minute so the shown settings stay roughly current.
//!
//! **Power off into Bluetooth** ([`Settings::bluetooth_on_power_off`]): the speaker starts on the
//! input it was turned off in, and on AUX or USB it may not be reachable over Bluetooth until
//! something connects. So when the app turns the speaker off, it first notes the input and volume,
//! switches to Bluetooth (waiting for the speaker to confirm) and then turns it off. The next time
//! it connects and finds the speaker on Bluetooth, it switches back and sets the volume. If the
//! input is something else by then, someone chose it, and it is left alone.
//!
//! **Volume per input** ([`Settings::remember_volume`]): the level is remembered for each input
//! (except Bluetooth, whose level follows the connected phone or computer), and switching to an
//! input restores its level. Only switches the app makes, or sees happen live, count: a different
//! input found when (re)connecting just becomes the new starting point, so a poll never makes the
//! volume jump.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::time::{interval, sleep, sleep_until, timeout, Instant, MissedTickBehavior};

use crate::proto::{self as p, CustomEq, FrameBuffer};
use crate::state::{Resume, State};

const SCAN_TIME: Duration = Duration::from_secs(6);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const READ_SPACING: Duration = Duration::from_millis(100); // the official app spaces these out too
const LINK_POLL: Duration = Duration::from_secs(1);
const MIN_DELAY: Duration = Duration::from_secs(2);
const MAX_DELAY: Duration = Duration::from_secs(20);

// Sharing mode
/// Keep the link this long after the last command (so a burst of clicks or a drag stays fast).
const HOLD_AFTER_USE: Duration = Duration::from_secs(20);
/// ...and this long after the UI stops being busy (popup or editor closed).
const HOLD_AFTER_BUSY: Duration = Duration::from_secs(10);
/// After a poll's reads are sent, wait this long for the replies before letting go.
const HOLD_AFTER_POLL: Duration = Duration::from_millis(1500);
/// How often to look in on the speaker while released.
const POLL_EVERY: Duration = Duration::from_secs(60);
const MAX_QUEUED: usize = 32;

// Volume per input
/// After an input switch, ignore level changes this long (the speaker or a Bluetooth source may
/// announce its own level) and put ours back once if it gets overridden.
const SETTLE_AFTER_SWITCH: Duration = Duration::from_secs(2);
/// Input changes seen this soon after the initial reads are still the starting point, not a switch.
const LIVE_AFTER_READS: Duration = Duration::from_millis(500);

// Power off into Bluetooth
/// How long to wait for the speaker to confirm the switch to Bluetooth before turning it off.
const CONFIRM_BLUETOOTH: Duration = Duration::from_secs(3);
/// While waiting for the speaker to come back on, look for it this often.
const LOOK_WHILE_OFF: Duration = Duration::from_secs(10);
const STATUS_OFF: &str = "Speaker turned off";

const STATUS_FREE: &str = "Speaker free for other devices";
const STATUS_UNAVAILABLE: &str = "Speaker unavailable (off, or in use by another device)";

/// Read once after connecting, in this order.
const INITIAL_READS: [u8; 8] = [
    p::CMD_GET_VOLUME,
    p::CMD_INPUT_GET,
    p::CMD_EQ_GET,
    p::CMD_BACKLIGHT_GET,
    p::CMD_PROMPT_GET,
    p::CMD_CUSTOM_EQ_GET,
    p::CMD_FIRMWARE_GET,
    p::CMD_NAME,
];

/// One device seen while scanning.
#[derive(Debug, Clone)]
pub struct Advert {
    /// Platform identifier: a MAC address on Windows, an opaque UUID on macOS.
    pub id: String,
    pub services: Vec<u128>,
    pub manufacturer_ids: Vec<u16>,
    pub rssi: Option<i16>,
}

pub trait Radio: Send + 'static {
    type Link: Link;
    /// Scan for up to `timeout`, returning early once `prefer` has been seen.
    fn scan(
        &mut self,
        timeout: Duration,
        prefer: Option<&str>,
    ) -> impl Future<Output = Result<Vec<Advert>, String>> + Send;
    /// Connect, find the characteristics and subscribe to notifications.
    fn connect(&mut self, id: &str) -> impl Future<Output = Result<Self::Link, String>> + Send;
}

pub trait Link: Send + 'static {
    fn write(&mut self, data: &[u8]) -> impl Future<Output = Result<(), String>> + Send;
    /// Next notification chunk, or `None` once the link is gone. Must be cancel-safe.
    fn next_data(&mut self) -> impl Future<Output = Option<Vec<u8>>> + Send;
    fn is_connected(&mut self) -> impl Future<Output = bool> + Send;
    fn close(&mut self) -> impl Future<Output = ()> + Send;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    SetVolume(u8),
    StepVolume(i8),
    SetInput(u8),
    SetEq(u8),
    /// Put this profile on the speaker and switch to it (EQ preset Custom).
    ApplyCustomEq(CustomEq),
    SetBacklightDuration(u8),
    SetBacklightSensitivity(u8),
    SetBtPrompt(bool),
    /// Play/pause/next/previous; only sent while the input is Bluetooth.
    Media(u8),
    PowerOff,
    /// Drop the current link (if any) and search again right away.
    Reconnect,
    /// Sharing mode on or off (see the module docs).
    SetSharing(bool),
    /// The UI is in use (popup or editor open): in sharing mode, connect and hold the link.
    SetBusy(bool),
    /// Remember and restore the volume for each input (see the module docs).
    SetVolumeMemory(bool),
    /// Power off into Bluetooth on or off (see the module docs).
    SetBluetoothOnPowerOff(bool),
    /// The user is about to do something (e.g. hovering the tray icon): connect now if released.
    Wake,
    Quit,
}

/// Receives the remembered level for each input (input id -> volume) whenever one changes.
pub type OnInputVolumes = Box<dyn FnMut(&HashMap<u8, u8>) + Send>;

/// Receives the input to put back after power off (or `None` once done), so it survives a restart.
pub type OnResume = Box<dyn FnMut(Option<Resume>) + Send>;

pub struct Settings {
    /// Device id from a previous run; tried first.
    pub remembered: Option<String>,
    /// Called after every successful connection so the caller can persist the id.
    pub on_connected: Box<dyn FnMut(&str) + Send>,
    /// Start in sharing mode.
    pub share: bool,
    /// Remember and restore the volume for each input.
    pub remember_volume: bool,
    /// Levels remembered from a previous run (input id -> volume).
    pub input_volumes: HashMap<u8, u8>,
    /// Called whenever a remembered level changes, so the caller can persist them.
    pub on_input_volumes: OnInputVolumes,
    /// Power off into Bluetooth, and put the input back when the speaker is next on.
    pub bluetooth_on_power_off: bool,
    /// An input still to put back from a previous run.
    pub resume: Option<Resume>,
    pub on_resume: OnResume,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            remembered: None,
            on_connected: Box::new(|_| {}),
            share: false,
            remember_volume: false,
            input_volumes: HashMap::new(),
            on_input_volumes: Box::new(|_| {}),
            bluetooth_on_power_off: false,
            resume: None,
            on_resume: Box::new(|_| {}),
        }
    }
}

#[derive(Default)]
struct VolumeMemory {
    enabled: bool,
    levels: HashMap<u8, u8>,
    /// Set after a switch: until then level changes are not remembered.
    settle_until: Option<Instant>,
    /// The level we restored for the input we switched to.
    target: Option<u8>,
    retried: bool,
    /// The input we just told the speaker to switch to (so that change counts as a switch).
    commanded: Option<u8>,
}

#[derive(Clone)]
pub struct Handle {
    cmd_tx: mpsc::UnboundedSender<Command>,
    state_rx: watch::Receiver<State>,
}

impl Handle {
    pub fn send(&self, cmd: Command) {
        let _ = self.cmd_tx.send(cmd); // the controller having stopped is not an error for callers
    }
    pub fn state(&self) -> State {
        self.state_rx.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<State> {
        self.state_rx.clone()
    }
}

pub struct Controller<R: Radio> {
    radio: R,
    settings: Settings,
    cmd_rx: mpsc::UnboundedReceiver<Command>,
    state_tx: watch::Sender<State>,
    sharing: bool,
    busy: bool,
    /// Commands waiting for a connection (sharing mode only).
    queue: VecDeque<Command>,
    vm: VolumeMemory,
    bluetooth_on_power_off: bool,
    resume: Option<Resume>,
    /// We turned the speaker off; cleared when it is reachable again.
    powered_off: bool,
}

enum End {
    Quit,
    Lost,
    Reconnect,
    /// Sharing mode: let go because nothing needs the speaker.
    Released,
}

/// Why we are connecting (decides how long to hold the link in sharing mode).
#[derive(Clone, Copy, PartialEq)]
enum Reason {
    Use,
    Poll,
}

impl<R: Radio> Controller<R> {
    pub fn new(radio: R, settings: Settings) -> (Self, Handle) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let sharing = settings.share;
        let vm = VolumeMemory {
            enabled: settings.remember_volume,
            levels: settings.input_volumes.clone(),
            ..VolumeMemory::default()
        };
        let (bluetooth_on_power_off, resume) = (settings.bluetooth_on_power_off, settings.resume);
        let state = State { sharing, volume_memory: vm.enabled, bluetooth_on_power_off, resume, ..State::default() };
        let (state_tx, state_rx) = watch::channel(state);
        let ctrl = Controller {
            radio,
            settings,
            cmd_rx,
            state_tx,
            sharing,
            busy: false,
            queue: VecDeque::new(),
            vm,
            bluetooth_on_power_off,
            resume,
            powered_off: false,
        };
        (ctrl, Handle { cmd_tx, state_rx })
    }

    /// Remember `volume` as the level for `input` (Bluetooth is left to the connected device).
    fn remember(&mut self, input: u8, volume: u8) {
        if !self.vm.enabled || input == p::INPUT_BLUETOOTH || self.vm.levels.get(&input) == Some(&volume) {
            return;
        }
        self.vm.levels.insert(input, volume);
        (self.settings.on_input_volumes)(&self.vm.levels);
    }

    /// Follow input and volume changes for the volume-per-input feature. `before` is (input,
    /// volume) before the latest event; `live` says the initial reads are over.
    async fn track(&mut self, link: &mut R::Link, before: (Option<u8>, Option<u8>), live: bool) {
        let st = self.snapshot();
        let (input, volume) = (st.input, st.volume);
        if input != before.0 {
            if let Some(i) = input {
                let ours = self.vm.commanded.take() == Some(i);
                if before.0.is_some() && (ours || live) {
                    self.switched_to(link, i, volume, st.max_volume).await;
                } else if let Some(v) = volume {
                    self.remember(i, v); // starting point: whatever it is now
                }
            }
            return;
        }
        if volume == before.1 {
            return;
        }
        let Some(v) = volume else { return };
        if self.vm.settle_until.is_some_and(|t| Instant::now() < t) {
            if let Some(t) = self.vm.target {
                if v != t && !self.vm.retried {
                    self.vm.retried = true;
                    log::info!("volume was changed to {v} right after the input switch; putting back {t}");
                    self.set_volume(link, t, st.max_volume).await;
                }
            }
        } else if let Some(i) = input {
            self.remember(i, v);
        }
    }

    /// The user changed the volume: that is the level for this input now, even right after a
    /// switch (so it is not "put back").
    fn user_set_volume(&mut self) {
        self.vm.settle_until = None;
        self.vm.target = None;
    }

    async fn switched_to(&mut self, link: &mut R::Link, input: u8, volume: Option<u8>, max: u8) {
        self.vm.settle_until = Some(Instant::now() + SETTLE_AFTER_SWITCH);
        self.vm.target = None;
        self.vm.retried = false;
        if !self.vm.enabled || input == p::INPUT_BLUETOOTH {
            return;
        }
        if let Some(&v) = self.vm.levels.get(&input) {
            self.vm.target = Some(v);
            if volume != Some(v) {
                log::info!("input {input}: restoring its volume {v}");
                self.set_volume(link, v, max).await;
            }
        }
    }

    /// Apply SetSharing / SetBusy. Returns false for any other command.
    fn control(&mut self, cmd: &Command) -> bool {
        match cmd {
            Command::SetSharing(on) => {
                self.sharing = *on;
                if !on {
                    self.queue.clear(); // queued commands are only for sharing mode
                }
                let on = *on;
                self.modify(|s| s.sharing = on);
                true
            }
            Command::SetBusy(b) => {
                self.busy = *b;
                true
            }
            Command::SetVolumeMemory(on) => {
                self.vm.enabled = *on;
                let on = *on;
                self.modify(|s| s.volume_memory = on);
                true
            }
            Command::SetBluetoothOnPowerOff(on) => {
                self.bluetooth_on_power_off = *on;
                let on = *on;
                self.modify(|s| s.bluetooth_on_power_off = on);
                if !on && self.resume.is_some() {
                    self.set_resume(None); // turned off: do not switch inputs behind the user's back
                }
                true
            }
            _ => false,
        }
    }

    fn set_resume(&mut self, resume: Option<Resume>) {
        self.resume = resume;
        (self.settings.on_resume)(resume);
        self.modify(|s| s.resume = resume);
    }

    /// After the app turned the speaker off: nothing to show until it is back.
    fn turned_off(&self) {
        self.modify(|s| {
            s.status = STATUS_OFF.into();
            s.clear_device();
        });
    }

    /// Wait (handling the speaker's messages) until it reports `input`. False on timeout or loss.
    async fn await_input(&mut self, link: &mut R::Link, input: u8, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        let mut frames = FrameBuffer::new();
        while self.snapshot().input != Some(input) {
            match timeout(deadline.saturating_duration_since(Instant::now()), link.next_data()).await {
                Ok(Some(chunk)) => {
                    for f in frames.push(&chunk) {
                        self.modify(|s| {
                            s.apply(&f);
                        });
                    }
                }
                Ok(None) | Err(_) => return false,
            }
        }
        true
    }

    /// Queue a command until we are connected. A newer volume replaces an older one, so a slider
    /// drag while connecting sends only where it ended.
    fn enqueue(&mut self, cmd: Command) {
        if matches!(cmd, Command::SetVolume(_) | Command::StepVolume(_)) {
            self.queue.retain(|c| !matches!(c, Command::SetVolume(_) | Command::StepVolume(_)));
        }
        if self.queue.len() >= MAX_QUEUED {
            self.queue.pop_front();
        }
        self.queue.push_back(cmd);
    }

    /// Sharing mode, not connected: show the last known settings with this status.
    fn released(&self, status: &str) {
        let status = status.to_string();
        self.modify(|s| {
            s.connected = false;
            if s.volume.is_some() {
                s.idle = true;
                s.status = status;
            } else {
                // nothing known yet: there are no "last known settings" to show
                s.status = status;
                s.clear_device();
            }
        });
    }

    fn modify(&self, f: impl FnOnce(&mut State)) {
        self.state_tx.send_if_modified(|s| {
            let before = s.clone();
            f(s);
            *s != before
        });
    }

    fn snapshot(&self) -> State {
        self.state_tx.borrow().clone()
    }

    /// Runs until [`Command::Quit`] (or every [`Handle`] is dropped).
    pub async fn run(mut self) {
        let mut delay = MIN_DELAY;
        // The first connection learns the speaker's settings; in sharing mode it then lets go.
        let mut reason = if self.sharing { Reason::Poll } else { Reason::Use };
        loop {
            match self.connect().await {
                Ok((id, mut link)) => {
                    log::info!("connected to {id}");
                    self.powered_off = false;
                    (self.settings.on_connected)(&id);
                    self.settings.remembered = Some(id);
                    delay = MIN_DELAY;
                    let end = self.session(&mut link, reason).await;
                    link.close().await;
                    reason = Reason::Use;
                    match end {
                        End::Quit => break,
                        End::Reconnect => self.modify(|s| s.connected = false),
                        End::Released => {
                            log::info!("released the speaker for other devices");
                            self.released(STATUS_FREE);
                            match self.idle().await {
                                Some(r) => reason = r,
                                None => break,
                            }
                        }
                        End::Lost if self.powered_off => {
                            self.turned_off();
                            if self.sharing {
                                match self.idle().await {
                                    Some(r) => reason = r,
                                    None => break,
                                }
                            } else if self.wait(MIN_DELAY).await {
                                break;
                            }
                        }
                        End::Lost if self.sharing => {
                            self.released("Disconnected - showing last known settings");
                            match self.idle().await {
                                Some(r) => reason = r,
                                None => break,
                            }
                        }
                        End::Lost => {
                            self.modify(|s| {
                                s.status = "Disconnected - retrying...".into();
                                s.clear_device();
                            });
                            if self.wait(delay).await {
                                break;
                            }
                        }
                    }
                }
                Err(msg) if self.sharing => {
                    log::info!("{msg}");
                    if !self.queue.is_empty() {
                        log::warn!("dropping {} queued command(s): the speaker could not be reached", self.queue.len());
                        self.queue.clear();
                    }
                    let known = self.snapshot().volume.is_some();
                    if self.powered_off {
                        self.turned_off();
                    } else {
                        self.released(if known { STATUS_UNAVAILABLE } else { msg.as_str() });
                    }
                    match self.idle().await {
                        Some(r) => reason = r,
                        None => break,
                    }
                }
                Err(msg) => {
                    log::info!("{msg}");
                    if self.powered_off {
                        self.turned_off();
                    } else {
                        self.modify(|s| {
                            s.status = msg;
                            s.clear_device();
                        });
                    }
                    if self.wait(delay).await {
                        break;
                    }
                    // waiting for the speaker to come back on: keep looking often
                    let max = if self.powered_off || self.resume.is_some() { LOOK_WHILE_OFF } else { MAX_DELAY };
                    delay = (delay * 2).min(max);
                }
            }
        }
        self.modify(|s| {
            s.status = "Stopped".into();
            s.clear_device();
        });
    }

    /// Find the speaker and connect (with a time limit). The error is a status line for the user.
    async fn connect(&mut self) -> Result<(String, R::Link), String> {
        self.modify(|s| s.connecting = true);
        let result = self.try_connect().await;
        self.modify(|s| s.connecting = false);
        result
    }

    async fn try_connect(&mut self) -> Result<(String, R::Link), String> {
        let first = self.snapshot().volume.is_none();
        self.modify(|s| s.status = if first { "Searching for the speaker...".into() } else { "Connecting...".into() });
        let id = match self.find().await {
            Ok(Some(id)) => id,
            Ok(None) => return Err("Speaker not found - switched off, or in use by another app or user?".into()),
            Err(e) => return Err(format!("Bluetooth problem: {e}")),
        };
        self.modify(|s| s.status = "Connecting...".into());
        match timeout(CONNECT_TIMEOUT, self.radio.connect(&id)).await {
            Ok(Ok(link)) => Ok((id, link)),
            Ok(Err(e)) => {
                log::warn!("connect failed: {e}");
                Err(format!("Connection failed: {e}"))
            }
            Err(_) => Err("Connection failed: timed out".into()),
        }
    }

    /// Sharing mode, released: wait until something needs the speaker. `None` means quit.
    async fn idle(&mut self) -> Option<Reason> {
        // waiting for the speaker to come back on (to put its input back): look more often
        let every = if self.powered_off || self.resume.is_some() { LOOK_WHILE_OFF } else { POLL_EVERY };
        let timer = sleep(every);
        tokio::pin!(timer);
        loop {
            if !self.sharing || self.busy || !self.queue.is_empty() {
                return Some(Reason::Use);
            }
            tokio::select! {
                _ = &mut timer => return Some(Reason::Poll),
                cmd = self.cmd_rx.recv() => match cmd {
                    None | Some(Command::Quit) => return None,
                    Some(Command::Reconnect | Command::Wake) => return Some(Reason::Use),
                    Some(c) => {
                        if !self.control(&c) {
                            self.enqueue(c); // the loop above then connects
                        }
                    }
                },
            }
        }
    }

    async fn find(&mut self) -> Result<Option<String>, String> {
        let prefer = self.settings.remembered.clone();
        let adverts = self.radio.scan(SCAN_TIME, prefer.as_deref()).await?;
        if let Some(r) = &prefer {
            if adverts.iter().any(|a| &a.id == r) {
                return Ok(Some(r.clone()));
            }
        }
        Ok(adverts
            .iter()
            .filter_map(|a| {
                let score = p::match_score(&a.services, &a.manufacturer_ids);
                (score > 0).then(|| ((score, a.rssi.unwrap_or(i16::MIN)), &a.id))
            })
            .max_by_key(|(k, _)| *k)
            .map(|(_, id)| id.clone()))
    }

    /// Always-connected mode, after a failure: sleep for `dur`, ending early on Reconnect or when
    /// sharing is switched on. Returns true if we should quit. Other commands are dropped: there is
    /// nothing to send them to.
    async fn wait(&mut self, dur: Duration) -> bool {
        let timer = sleep(dur);
        tokio::pin!(timer);
        loop {
            tokio::select! {
                _ = &mut timer => return false,
                cmd = self.cmd_rx.recv() => match cmd {
                    None | Some(Command::Quit) => return true,
                    Some(Command::Reconnect) => return false,
                    Some(c) => {
                        if self.control(&c) && self.sharing {
                            return false; // sharing has its own (quiet) way of retrying
                        }
                    }
                },
            }
        }
    }

    async fn session(&mut self, link: &mut R::Link, reason: Reason) -> End {
        // Commands that arrived while we were connecting: in sharing mode they are why we
        // connected; otherwise they are stale clicks and are dropped. Never miss a Quit.
        while let Ok(c) = self.cmd_rx.try_recv() {
            match c {
                Command::Quit => return End::Quit,
                Command::Reconnect | Command::Wake => {}
                c => {
                    if !self.control(&c) && self.sharing {
                        self.enqueue(c);
                    }
                }
            }
        }
        if !self.sharing {
            self.queue.clear();
        }
        self.modify(|s| {
            s.connected = true;
            s.idle = false;
            s.status = "Connected".into();
        });

        self.vm.settle_until = None;
        self.vm.target = None;
        self.vm.commanded = None;

        // Queued commands first (the user is waiting for them), then the reads.
        let queued: Vec<Command> = self.queue.drain(..).collect();
        for c in queued {
            let st = self.snapshot();
            self.handle(link, c).await;
            self.track(link, (st.input, st.volume), false).await;
        }

        let mut frames = FrameBuffer::new();
        let mut pending: VecDeque<u8> = INITIAL_READS.into();
        let mut next_read = Instant::now();
        let mut reads_done = Instant::now();
        let mut hold_until = Instant::now() + if reason == Reason::Use { HOLD_AFTER_USE } else { Duration::ZERO };
        let mut poll = interval(LINK_POLL);
        poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut resume_checked = false;

        loop {
            // Power off into Bluetooth: back on, so put its input back (once the reads are in).
            if !resume_checked && pending.is_empty() && Instant::now() >= reads_done + LIVE_AFTER_READS {
                if let Some(r) = self.resume {
                    match self.snapshot().input {
                        Some(p::INPUT_BLUETOOTH) => {
                            resume_checked = true;
                            log::info!("speaker is back on: switching to input {} as before the power off", r.input);
                            self.set_resume(None);
                            self.handle(link, Command::SetInput(r.input)).await;
                            self.vm.commanded = None;
                            if let Some(v) = r.volume {
                                self.handle(link, Command::SetVolume(v)).await;
                            }
                            hold_until = hold_until.max(Instant::now() + HOLD_AFTER_POLL);
                        }
                        Some(other) => {
                            resume_checked = true;
                            log::info!("speaker came back on input {other}, not Bluetooth: leaving it as it is");
                            self.set_resume(None);
                        }
                        None => {} // the input is not known yet
                    }
                } else {
                    resume_checked = true;
                }
            }
            let releasable = self.sharing && !self.busy && pending.is_empty();
            let release_at = hold_until.max(reads_done + HOLD_AFTER_POLL);
            let settle_at = self.vm.settle_until.unwrap_or_else(Instant::now);
            let settling = self.vm.settle_until.is_some();
            let before = {
                let s = self.snapshot();
                (s.input, s.volume)
            };
            tokio::select! {
                data = link.next_data() => match data {
                    Some(chunk) => {
                        for f in frames.push(&chunk) {
                            self.modify(|s| { s.apply(&f); });
                        }
                    }
                    None => return End::Lost,
                },
                cmd = self.cmd_rx.recv() => match cmd {
                    None | Some(Command::Quit) => return End::Quit,
                    Some(Command::Reconnect) => return End::Reconnect,
                    Some(Command::Wake) => hold_until = hold_until.max(Instant::now() + HOLD_AFTER_USE),
                    Some(c) if self.control(&c) => {
                        let extra = match c {
                            Command::SetBusy(false) => HOLD_AFTER_BUSY,
                            _ => HOLD_AFTER_USE,
                        };
                        hold_until = hold_until.max(Instant::now() + extra);
                    }
                    Some(c) => {
                        self.handle(link, c).await;
                        hold_until = hold_until.max(Instant::now() + HOLD_AFTER_USE);
                    }
                },
                _ = sleep_until(next_read), if !pending.is_empty() => {
                    if let Some(cmd) = pending.pop_front() {
                        self.send(link, p::build_frame(cmd, &[])).await;
                    }
                    next_read = Instant::now() + READ_SPACING;
                    if pending.is_empty() {
                        reads_done = Instant::now();
                    }
                },
                // wake up once the reads are in, to put the input back (see the top of the loop)
                _ = sleep_until(reads_done + LIVE_AFTER_READS), if !resume_checked && self.resume.is_some() && pending.is_empty() => {},
                _ = sleep_until(release_at), if releasable && (resume_checked || self.resume.is_none()) => return End::Released,
                _ = sleep_until(settle_at), if settling => {
                    // the switch has settled: from now on, level changes belong to this input
                    self.vm.settle_until = None;
                    self.vm.target = None;
                    let s = self.snapshot();
                    if let (Some(i), Some(v)) = (s.input, s.volume) {
                        self.remember(i, v);
                    }
                },
                _ = poll.tick() => {
                    if !link.is_connected().await {
                        return End::Lost;
                    }
                },
            }
            let live = pending.is_empty() && Instant::now() >= reads_done + LIVE_AFTER_READS;
            self.track(link, before, live).await;
        }
    }

    async fn send(&mut self, link: &mut R::Link, frame: Vec<u8>) -> bool {
        match timeout(WRITE_TIMEOUT, link.write(&frame)).await {
            Ok(Ok(())) => true,
            Ok(Err(e)) => {
                log::warn!("write failed: {e}");
                false
            }
            Err(_) => {
                log::warn!("write timed out");
                false
            }
        }
    }

    async fn handle(&mut self, link: &mut R::Link, cmd: Command) {
        let st = self.snapshot();
        match cmd {
            Command::SetVolume(n) => {
                self.user_set_volume();
                self.set_volume(link, n, st.max_volume).await;
            }
            Command::StepVolume(d) => {
                self.user_set_volume();
                if let Some(v) = st.volume {
                    let n = (i16::from(v) + i16::from(d)).clamp(0, i16::from(st.max_volume));
                    self.set_volume(link, n as u8, st.max_volume).await;
                }
            }
            Command::SetInput(n) => {
                if p::is_valid(p::INPUTS, n)
                    && self.send(link, p::build_frame(p::CMD_INPUT_SET, &[p::INPUT_KEY, n])).await
                {
                    self.vm.commanded = Some(n);
                    self.modify(|s| s.input = Some(n)); // the speaker's 0x61 notification confirms or corrects
                }
            }
            Command::SetEq(n) => {
                if p::is_valid(p::EQ_PRESETS, n) && self.send(link, p::build_frame(p::CMD_EQ_SET, &[n])).await {
                    self.modify(|s| s.eq_preset = Some(n));
                }
            }
            Command::ApplyCustomEq(eq) => {
                if eq.bands.len() != p::BAND_COUNT || eq.name.is_empty() {
                    return;
                }
                // Same order as the phone app: select Custom first, then write the profile.
                if st.eq_preset != Some(p::EQ_CUSTOM) {
                    if !self.send(link, p::build_frame(p::CMD_EQ_SET, &[p::EQ_CUSTOM])).await {
                        return;
                    }
                    self.modify(|s| s.eq_preset = Some(p::EQ_CUSTOM));
                }
                if self.send(link, p::build_frame(p::CMD_CUSTOM_EQ_SET, &p::encode_custom_eq(&eq))).await {
                    self.modify(|s| s.custom_eq = Some(eq)); // the speaker echoes it back
                }
            }
            // The speaker takes both backlight fields in one command, so the field you did not
            // change is sent as last reported; if that is not known yet, nothing is sent.
            Command::SetBacklightDuration(d) => self.set_backlight(link, Some(d), st.backlight_sensitivity).await,
            Command::SetBacklightSensitivity(s) => self.set_backlight(link, st.backlight_duration, Some(s)).await,
            Command::SetBtPrompt(on) => {
                let frame = p::build_frame(p::CMD_PROMPT_SET, &[p::PROMPT_KEY, p::PROMPT_BT_CONNECTION, u8::from(on)]);
                if self.send(link, frame).await {
                    self.modify(|s| s.bt_prompt = Some(on));
                }
            }
            Command::Media(a) => {
                // Observed: media control only does anything on the Bluetooth input.
                if p::is_valid(p::MEDIA_ACTIONS, a) && st.input == Some(p::INPUT_BLUETOOTH) {
                    self.send(link, p::build_frame(p::CMD_MEDIA, &[a])).await;
                }
            }
            Command::PowerOff => {
                if self.bluetooth_on_power_off {
                    if let Some(input) = st.input.filter(|i| *i != p::INPUT_BLUETOOTH) {
                        self.set_resume(Some(Resume { input, volume: st.volume }));
                        log::info!("power off: switching to Bluetooth first (was input {input})");
                        let frame = p::build_frame(p::CMD_INPUT_SET, &[p::INPUT_KEY, p::INPUT_BLUETOOTH]);
                        if self.send(link, frame).await
                            && !self.await_input(link, p::INPUT_BLUETOOTH, CONFIRM_BLUETOOTH).await
                        {
                            log::warn!("the speaker did not confirm Bluetooth; turning it off anyway");
                        }
                    }
                }
                if self.send(link, p::build_frame(p::CMD_POWER_OFF, &[])).await {
                    self.powered_off = true;
                }
            }
            // handled by the caller
            Command::Reconnect
            | Command::Quit
            | Command::SetSharing(_)
            | Command::SetBusy(_)
            | Command::SetVolumeMemory(_)
            | Command::SetBluetoothOnPowerOff(_)
            | Command::Wake => {}
        }
    }

    async fn set_volume(&mut self, link: &mut R::Link, n: u8, max: u8) {
        let n = n.min(max);
        if self.send(link, p::build_frame(p::CMD_SET_VOLUME, &[n])).await {
            self.modify(|s| s.volume = Some(n));
        }
    }

    async fn set_backlight(&mut self, link: &mut R::Link, duration: Option<u8>, sensitivity: Option<u8>) {
        let (Some(d), Some(s)) = (duration, sensitivity) else { return };
        if !p::is_valid(p::BACKLIGHT_DURATIONS, d) || !p::is_valid(p::BACKLIGHT_SENSITIVITIES, s) {
            return;
        }
        if self.send(link, p::build_frame(p::CMD_BACKLIGHT_SET, &[p::BACKLIGHT_KEY, d, s])).await {
            self.modify(|st| {
                st.backlight_duration = Some(d);
                st.backlight_sensitivity = Some(s);
            });
        }
    }
}

#[cfg(test)]
mod tests;
