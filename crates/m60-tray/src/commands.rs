//! What the popup page can ask for (`invoke`) and the state snapshot it receives (`state` event).

use std::sync::Arc;

use m60_core::{proto, Command, Handle, State};
use serde::Serialize;
use tauri::AppHandle;

use crate::config::Preset;
use crate::presets::{self, Store};
use crate::{popup, tray, version};

/// A choice in the settings page: value and label.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Choice {
    pub id: u8,
    pub name: &'static str,
}

fn choices(table: proto::Table) -> Vec<Choice> {
    table.iter().map(|(id, name)| Choice { id: *id, name }).collect()
}

/// What the Settings window shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub start_at_login: bool,
    pub share: bool,
    pub remember_volume: bool,
    pub bluetooth_on_power_off: bool,
    /// While waiting to put an input back after a power off: its name (e.g. "AUX").
    pub resume_input: Option<&'static str>,
    pub connected: bool,
    /// The speaker settings can be changed (connected, or sharing and it will connect).
    pub controllable: bool,
    pub backlight_duration: Option<u8>,
    pub backlight_sensitivity: Option<u8>,
    pub bt_prompt: Option<bool>,
    pub durations: Vec<Choice>,
    pub sensitivities: Vec<Choice>,
    /// About: the speaker's model and firmware (when known) and this app's version and build time.
    pub model: Option<String>,
    pub firmware: Option<String>,
    pub app_version: &'static str,
    pub build_time: &'static str,
}

impl SettingsDto {
    pub fn new(start_at_login: bool, s: &State) -> Self {
        SettingsDto {
            start_at_login,
            share: s.sharing,
            remember_volume: s.volume_memory,
            bluetooth_on_power_off: s.bluetooth_on_power_off,
            resume_input: s.resume.and_then(|r| proto::lookup(proto::INPUTS, r.input)),
            connected: s.connected,
            controllable: s.controllable(),
            backlight_duration: s.backlight_duration,
            backlight_sensitivity: s.backlight_sensitivity,
            bt_prompt: s.bt_prompt,
            durations: choices(proto::BACKLIGHT_DURATIONS),
            sensitivities: choices(proto::BACKLIGHT_SENSITIVITIES),
            model: s.model.clone(),
            firmware: s.firmware.clone(),
            app_version: version::APP_VERSION,
            build_time: version::BUILD_TIME,
        }
    }
}

#[tauri::command]
pub fn get_settings(app: AppHandle, handle: tauri::State<'_, Handle>) -> SettingsDto {
    SettingsDto::new(tray::start_at_login(&app), &handle.state())
}

/// The Settings page reports how tall its content is; the window is sized to fit it.
#[tauri::command]
pub fn fit_settings(app: AppHandle, height: f64) {
    crate::settings_window::fit(&app, height);
}

/// Change one of the app's own settings (saved in config.json, or the login item for startAtLogin).
#[tauri::command]
pub fn set_app_setting(
    app: AppHandle,
    store: tauri::State<'_, Arc<Store>>,
    handle: tauri::State<'_, Handle>,
    key: String,
    on: bool,
) -> Result<(), String> {
    match key.as_str() {
        "startAtLogin" => tray::set_start_at_login(&app, on),
        "share" => {
            store.set_sharing(on);
            handle.send(Command::SetSharing(on));
        }
        "rememberVolume" => {
            store.set_remember_volume(on);
            handle.send(Command::SetVolumeMemory(on));
        }
        "bluetoothOnPowerOff" => {
            store.set_bluetooth_on_power_off(on);
            handle.send(Command::SetBluetoothOnPowerOff(on));
        }
        other => return Err(format!("unknown setting {other}")),
    }
    log::info!("setting {key}: {}", if on { "on" } else { "off" });
    tray::refresh(&app);
    Ok(())
}

/// Intelligent Backlight: change the duration and/or the sensitivity.
#[tauri::command]
pub fn set_backlight(handle: tauri::State<'_, Handle>, duration: Option<u8>, sensitivity: Option<u8>) {
    if let Some(d) = duration {
        handle.send(Command::SetBacklightDuration(d));
    }
    if let Some(s) = sensitivity {
        handle.send(Command::SetBacklightSensitivity(s));
    }
}

/// Prompts and tones: the Bluetooth connection tone.
#[tauri::command]
pub fn set_bt_prompt(handle: tauri::State<'_, Handle>, on: bool) {
    handle.send(Command::SetBtPrompt(on));
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateDto {
    pub connected: bool,
    /// The controls can be used (connected, or sharing with known settings).
    pub controllable: bool,
    /// The connection status on its own, e.g. "Speaker free for other devices".
    pub status: String,
    pub status_line: String,
    pub volume: Option<u8>,
    pub max_volume: u8,
    /// e.g. "Bluetooth - EQ Music"
    pub detail: String,
    /// The speaker is on the Bluetooth input (media controls only work there).
    pub bluetooth: bool,
    /// The media buttons can be used: usable controls and the Bluetooth input.
    pub media_enabled: bool,
    /// What the Bluetooth source reports, or null when there is nothing to show / not heard yet.
    pub now_playing: Option<NowPlayingDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NowPlayingDto {
    pub title: String,
    pub artist: String,
}

impl From<&State> for StateDto {
    fn from(s: &State) -> Self {
        let mut detail = Vec::new();
        if let Some(name) = s.input.and_then(|i| proto::lookup(proto::INPUTS, i)) {
            detail.push(name.to_string());
        }
        if let Some(name) = s.eq_name() {
            detail.push(format!("EQ {name}"));
        }
        StateDto {
            connected: s.connected,
            controllable: s.controllable(),
            status: s.status.clone(),
            status_line: s.status_line(),
            volume: s.volume,
            max_volume: s.max_volume,
            detail: detail.join(" - "),
            bluetooth: s.input == Some(proto::INPUT_BLUETOOTH),
            media_enabled: s.controllable() && s.input == Some(proto::INPUT_BLUETOOTH),
            now_playing: s.now_playing.clone().flatten().map(|n| NowPlayingDto { title: n.title, artist: n.artist }),
        }
    }
}

/// What the EQ editor shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetsDto {
    pub presets: Vec<Preset>,
    /// The saved preset currently on the speaker and in use, if any.
    pub active_id: Option<u32>,
    pub connected: bool,
    /// A preset can be applied (connected, or sharing and it will connect).
    pub controllable: bool,
    pub freqs: [u16; 6],
    pub min_db: f32,
    pub max_db: f32,
    pub step_db: f32,
    pub max_name_bytes: usize,
}

impl PresetsDto {
    pub fn new(store: &Store, s: &State) -> Self {
        let active_id = match (s.eq_preset, &s.custom_eq) {
            (Some(proto::EQ_CUSTOM), Some(c)) => Some(c.id()),
            _ => None,
        };
        PresetsDto {
            presets: store.presets(),
            active_id,
            connected: s.connected,
            controllable: s.controllable(),
            freqs: proto::BAND_FREQS_HZ,
            min_db: proto::GAIN_MIN_DB,
            max_db: proto::GAIN_MAX_DB,
            step_db: proto::GAIN_STEP_DB,
            max_name_bytes: proto::MAX_PROFILE_NAME,
        }
    }
}

#[tauri::command]
pub fn get_presets(store: tauri::State<'_, Arc<Store>>, handle: tauri::State<'_, Handle>) -> PresetsDto {
    PresetsDto::new(&store, &handle.state())
}

#[tauri::command]
pub fn save_preset(
    app: AppHandle,
    store: tauri::State<'_, Arc<Store>>,
    handle: tauri::State<'_, Handle>,
    id: Option<u32>,
    name: String,
    gains: [f32; 6],
) -> Result<Preset, String> {
    let p = store.save(id, &name, gains, presets::now_secs())?;
    tray::presets_changed(&app, &handle);
    Ok(p)
}

#[tauri::command]
pub fn delete_preset(
    app: AppHandle,
    store: tauri::State<'_, Arc<Store>>,
    handle: tauri::State<'_, Handle>,
    id: u32,
) -> Result<(), String> {
    if !store.delete(id) {
        return Err("That preset no longer exists.".into());
    }
    tray::presets_changed(&app, &handle);
    Ok(())
}

/// Put a saved preset on the speaker (switching the EQ to Custom).
#[tauri::command]
pub fn apply_preset(
    store: tauri::State<'_, Arc<Store>>,
    handle: tauri::State<'_, Handle>,
    id: u32,
) -> Result<(), String> {
    if !handle.state().controllable() {
        return Err("The speaker is not connected. The preset is saved; apply it once the speaker is back.".into());
    }
    let p = store.get(id).ok_or("That preset no longer exists.")?;
    handle.send(Command::ApplyCustomEq(presets::to_profile(&p)));
    Ok(())
}

#[tauri::command]
pub fn get_state(handle: tauri::State<'_, Handle>) -> StateDto {
    StateDto::from(&handle.state())
}

#[tauri::command]
pub fn set_volume(handle: tauri::State<'_, Handle>, value: u8) {
    handle.send(Command::SetVolume(value)); // the controller clamps it to the speaker's maximum
}

/// Play / pause / next / previous on the Bluetooth source (see `proto::MEDIA_ACTIONS`).
#[tauri::command]
pub fn media(handle: tauri::State<'_, Handle>, action: u8) -> Result<(), String> {
    if proto::lookup(proto::MEDIA_ACTIONS, action).is_none() {
        return Err(format!("unknown media action {action}"));
    }
    handle.send(Command::Media(action)); // the controller only sends it on the Bluetooth input
    Ok(())
}

#[tauri::command]
pub fn hide_popup(app: AppHandle) {
    popup::hide(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dto_describes_the_state_for_the_page() {
        let s = State {
            connected: true,
            status: "Connected".into(),
            volume: Some(9),
            input: Some(3),
            eq_preset: Some(4),
            ..State::default()
        };
        let dto = StateDto::from(&s);
        assert_eq!(dto.status_line, "Connected - volume 9 - AUX - EQ Custom");
        assert_eq!(dto.detail, "AUX - EQ Custom");
        assert_eq!((dto.volume, dto.max_volume, dto.connected), (Some(9), 16, true));
    }

    #[test]
    fn dto_media_fields_follow_the_input() {
        use m60_core::proto::NowPlaying;
        let np = NowPlaying { title: "Song".into(), artist: "Band".into() };
        let mut s =
            State { connected: true, volume: Some(5), input: Some(1), now_playing: Some(Some(np)), ..State::default() };
        let dto = StateDto::from(&s);
        assert!(dto.bluetooth && dto.media_enabled);
        assert_eq!(dto.now_playing, Some(NowPlayingDto { title: "Song".into(), artist: "Band".into() }));
        s.input = Some(3);
        let dto = StateDto::from(&s);
        assert!(!dto.bluetooth && !dto.media_enabled, "off Bluetooth the buttons are off");
        s.input = Some(1);
        s.connected = false;
        assert!(!StateDto::from(&s).media_enabled, "and without a usable connection");
        s.now_playing = Some(None);
        assert_eq!(StateDto::from(&s).now_playing, None);
    }

    #[test]
    fn settings_dto_describes_app_and_speaker_settings() {
        use m60_core::Resume;
        let s = State {
            connected: true,
            volume: Some(5),
            sharing: true,
            volume_memory: true,
            bluetooth_on_power_off: true,
            resume: Some(Resume { input: 3, volume: Some(9) }),
            backlight_duration: Some(1),
            backlight_sensitivity: Some(2),
            bt_prompt: Some(false),
            model: Some("EDIFIER M60".into()),
            firmware: Some("2.4.1".into()),
            ..State::default()
        };
        let d = SettingsDto::new(true, &s);
        assert_eq!((d.model.as_deref(), d.firmware.as_deref()), (Some("EDIFIER M60"), Some("2.4.1")));
        assert_eq!(d.app_version, env!("CARGO_PKG_VERSION"));
        assert!(d.start_at_login && d.share && d.remember_volume && d.bluetooth_on_power_off && d.controllable);
        assert_eq!(d.resume_input, Some("AUX"));
        assert_eq!((d.backlight_duration, d.backlight_sensitivity, d.bt_prompt), (Some(1), Some(2), Some(false)));
        assert_eq!(d.durations.iter().map(|c| c.name).collect::<Vec<_>>(), ["5 s", "10 s", "20 s", "On"]);
        assert_eq!(d.sensitivities.iter().map(|c| c.name).collect::<Vec<_>>(), ["High", "General", "Low"]);
        let json = serde_json::to_value(&d).unwrap();
        for key in [
            "startAtLogin",
            "bluetoothOnPowerOff",
            "resumeInput",
            "backlightDuration",
            "btPrompt",
            "model",
            "firmware",
            "appVersion",
            "buildTime",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
    }

    #[test]
    fn dto_serialises_with_camel_case_keys() {
        let json = serde_json::to_value(StateDto::from(&State::default())).unwrap();
        for key in [
            "connected",
            "controllable",
            "status",
            "statusLine",
            "volume",
            "maxVolume",
            "detail",
            "bluetooth",
            "mediaEnabled",
            "nowPlaying",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
        assert!(json["volume"].is_null());
    }
}
