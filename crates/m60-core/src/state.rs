//! What we know about the speaker, updated from its notifications.

use crate::proto::{self as p, CustomEq, Frame, NowPlaying};

/// What to put back after the app turned the speaker off "into Bluetooth".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resume {
    pub input: u8,
    pub volume: Option<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// Human-readable connection status ("Searching...", "Connecting...", ...).
    pub status: String,
    pub connected: bool,
    pub volume: Option<u8>,
    pub input: Option<u8>,
    pub eq_preset: Option<u8>,
    pub backlight_duration: Option<u8>,
    pub backlight_sensitivity: Option<u8>,
    pub bt_prompt: Option<bool>,
    pub firmware: Option<String>,
    pub model: Option<String>,
    /// The custom EQ profile stored on the speaker (used while `eq_preset` is Custom).
    pub custom_eq: Option<CustomEq>,
    /// What the Bluetooth source reports as playing. `Some(None)`: nothing to show; `None`: not
    /// heard yet (the speaker pushes it; it cannot be asked for).
    pub now_playing: Option<Option<NowPlaying>>,
    pub max_volume: u8,
    /// Sharing mode: connect only when needed, so other devices can use the speaker in between.
    pub sharing: bool,
    /// Sharing mode and not connected: the device fields are the last known settings.
    pub idle: bool,
    /// The volume is remembered per input and restored on switching.
    pub volume_memory: bool,
    /// Looking for the speaker or connecting to it right now.
    pub connecting: bool,
    /// Power off from the app switches to Bluetooth first (so the speaker comes back on reachable).
    pub bluetooth_on_power_off: bool,
    /// The input (and volume) to put back when the speaker is next on, if any.
    pub resume: Option<Resume>,
}

impl Default for State {
    fn default() -> Self {
        State {
            status: "Starting...".into(),
            connected: false,
            volume: None,
            input: None,
            eq_preset: None,
            backlight_duration: None,
            backlight_sensitivity: None,
            bt_prompt: None,
            firmware: None,
            model: None,
            custom_eq: None,
            now_playing: None,
            max_volume: p::DEFAULT_MAX_VOLUME,
            sharing: false,
            idle: false,
            volume_memory: false,
            connecting: false,
            bluetooth_on_power_off: false,
            resume: None,
        }
    }
}

impl State {
    /// Forget everything read from the speaker (used when the link is gone).
    pub fn clear_device(&mut self) {
        let status = std::mem::take(&mut self.status);
        *self = State {
            status,
            max_volume: self.max_volume,
            sharing: self.sharing,
            volume_memory: self.volume_memory,
            bluetooth_on_power_off: self.bluetooth_on_power_off,
            resume: self.resume,
            ..State::default()
        };
    }

    /// Can the UI offer the controls? Connected, or (sharing) released with known settings: a
    /// change then connects, is sent, and the link is let go again later.
    pub fn controllable(&self) -> bool {
        self.connected || (self.sharing && self.idle && self.volume.is_some())
    }

    pub fn status_line(&self) -> String {
        let prefix = if self.connected { "Connected" } else { self.status.as_str() };
        match (self.connected || self.idle, self.volume) {
            (true, Some(v)) => {
                let mut parts = vec![format!("{prefix} - volume {v}")];
                if let Some(name) = self.input.and_then(|i| p::lookup(p::INPUTS, i)) {
                    parts.push(name.to_string());
                }
                if let Some(name) = self.eq_name() {
                    parts.push(format!("EQ {name}"));
                }
                parts.join(" - ")
            }
            _ => self.status.clone(),
        }
    }

    /// The EQ preset's name; for Custom, the custom profile's own name when it is known.
    pub fn eq_name(&self) -> Option<String> {
        match (self.eq_preset, &self.custom_eq) {
            (Some(p::EQ_CUSTOM), Some(c)) if !c.name.is_empty() => Some(c.name.clone()),
            (Some(i), _) => p::lookup(p::EQ_PRESETS, i).map(str::to_string),
            (None, _) => None,
        }
    }

    /// Apply one speaker frame. Returns true if anything changed. Unknown commands, odd lengths and
    /// values outside the known tables are ignored rather than trusted.
    pub fn apply(&mut self, f: &Frame) -> bool {
        let before = self.clone();
        let pl = f.payload.as_slice();
        match f.cmd {
            p::CMD_GET_VOLUME | p::CMD_SET_VOLUME if pl.len() == 2 => {
                if pl[0] != 0 {
                    self.max_volume = pl[0];
                }
                self.volume = Some(pl[1]);
            }
            p::CMD_INPUT_GET if pl.len() == 2 && pl[0] == p::INPUT_KEY => {
                if p::is_valid(p::INPUTS, pl[1]) {
                    self.input = Some(pl[1]);
                }
            }
            p::CMD_EQ_GET | p::CMD_EQ_SET if pl.len() == 1 => {
                if p::is_valid(p::EQ_PRESETS, pl[0]) {
                    self.eq_preset = Some(pl[0]);
                }
            }
            p::CMD_BACKLIGHT_GET | p::CMD_BACKLIGHT_SET if pl.len() == 3 && pl[0] == p::BACKLIGHT_KEY => {
                if p::is_valid(p::BACKLIGHT_DURATIONS, pl[1]) && p::is_valid(p::BACKLIGHT_SENSITIVITIES, pl[2]) {
                    self.backlight_duration = Some(pl[1]);
                    self.backlight_sensitivity = Some(pl[2]);
                }
            }
            p::CMD_PROMPT_GET if pl.len() == 2 && pl[0] == p::PROMPT_KEY && pl[1] <= 1 => {
                self.bt_prompt = Some(pl[1] == 1);
            }
            p::CMD_PROMPT_SET
                if pl.len() == 3 && pl[0] == p::PROMPT_KEY && pl[1] == p::PROMPT_BT_CONNECTION && pl[2] <= 1 =>
            {
                self.bt_prompt = Some(pl[2] == 1);
            }
            p::CMD_CUSTOM_EQ_GET => {
                if let Some(eq) = p::decode_custom_eq_reply(pl) {
                    self.custom_eq = Some(eq);
                }
            }
            p::CMD_CUSTOM_EQ_SET => {
                if let Some(eq) = p::decode_custom_eq(pl) {
                    self.custom_eq = Some(eq);
                }
            }
            p::CMD_NOW_PLAYING => {
                if let Some(np) = p::decode_now_playing(pl) {
                    self.now_playing = Some(np);
                }
            }
            p::CMD_FIRMWARE_GET if pl.len() == 3 => {
                self.firmware = Some(format!("{}.{}.{}", pl[0], pl[1], pl[2]));
            }
            p::CMD_NAME if !pl.is_empty() && pl.len() <= 64 => {
                let name = String::from_utf8_lossy(pl);
                let name = name.trim_matches(|c: char| c == '\0' || c.is_whitespace());
                if !name.is_empty() {
                    self.model = Some(name.to_string());
                }
            }
            _ => {}
        }
        *self != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(cmd: u8, payload: &[u8]) -> Frame {
        Frame { cmd, payload: payload.to_vec() }
    }

    #[test]
    fn applies_each_kind_of_reply() {
        let mut s = State::default();
        assert!(s.apply(&f(0x66, &[0x10, 7])));
        assert!(s.apply(&f(0x61, &[0x0F, 3])));
        assert!(s.apply(&f(0xD5, &[2])));
        assert!(s.apply(&f(0x84, &[3, 2, 0])));
        assert!(s.apply(&f(0x86, &[2, 1])));
        assert!(s.apply(&f(0xC6, &[2, 4, 1])));
        assert!(s.apply(&f(0xC9, b"EDIFIER M60")));
        assert_eq!(
            (s.volume, s.input, s.eq_preset, s.backlight_duration, s.backlight_sensitivity, s.bt_prompt),
            (Some(7), Some(3), Some(2), Some(2), Some(0), Some(true))
        );
        assert_eq!(s.firmware.as_deref(), Some("2.4.1"));
        assert_eq!(s.model.as_deref(), Some("EDIFIER M60"));
        assert_eq!(s.max_volume, 16);
    }

    #[test]
    fn echoes_of_setters_update_state_too() {
        let mut s = State::default();
        s.apply(&f(0x85, &[3, 3, 2]));
        assert_eq!((s.backlight_duration, s.backlight_sensitivity), (Some(3), Some(2)));
        s.apply(&f(0x87, &[2, 1, 0]));
        assert_eq!(s.bt_prompt, Some(false));
        s.apply(&f(0xC4, &[4]));
        assert_eq!(s.eq_preset, Some(4));
        s.apply(&f(0x67, &[0x10, 16]));
        assert_eq!(s.volume, Some(16));
    }

    #[test]
    fn invalid_values_are_ignored() {
        let mut s = State::default();
        assert!(!s.apply(&f(0x61, &[0x0F, 9])), "unknown input");
        assert!(!s.apply(&f(0x61, &[0x0E, 1])), "wrong key");
        assert!(!s.apply(&f(0xD5, &[9])), "unknown preset");
        assert!(!s.apply(&f(0x84, &[3, 9, 1])), "unknown duration");
        assert!(!s.apply(&f(0x84, &[4, 1, 1])), "wrong key");
        assert!(!s.apply(&f(0x86, &[2, 5])), "bad prompt value");
        assert!(!s.apply(&f(0x66, &[0x10])), "short volume reply");
        assert!(!s.apply(&f(0xC9, b"")), "empty name");
        assert!(!s.apply(&f(0x99, &[1, 2, 3])), "unknown command");
        assert_eq!(s, State::default());
    }

    #[test]
    fn repeated_identical_reply_reports_no_change() {
        let mut s = State::default();
        assert!(s.apply(&f(0x66, &[0x10, 7])));
        assert!(!s.apply(&f(0x66, &[0x10, 7])));
    }

    #[test]
    fn custom_profile_comes_from_43_and_46_and_names_the_eq() {
        let hex = |s: &str| {
            (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect::<Vec<u8>>()
        };
        let mut s = State { connected: true, volume: Some(5), eq_preset: Some(4), ..State::default() };
        assert_eq!(s.eq_name().as_deref(), Some("Custom"), "name unknown yet");
        assert!(s.apply(&f(
            0x43,
            &hex("03060000003e0007010000fa0007020003e8060703000fa0060704001f40060705003e8006073afbf26a47616d696e67")
        )));
        assert_eq!(s.custom_eq.as_ref().unwrap().name, "Gaming");
        assert_eq!(s.status_line(), "Connected - volume 5 - EQ Gaming");
        assert!(s.apply(&f(
            0x46,
            &hex("0000003e0707010000fa0807020003e8040703000fa0070704001f40060705003e80040792faf26a4d75736963")
        )));
        assert_eq!(s.eq_name().as_deref(), Some("Music"));
        s.eq_preset = Some(1);
        assert_eq!(s.eq_name().as_deref(), Some("Monitor"), "a built-in preset ignores the custom profile");
        assert!(!s.apply(&f(0x43, &[3, 6, 1, 2])), "garbage is ignored");
    }

    #[test]
    fn now_playing_follows_the_pushes() {
        let mut s = State::default();
        assert_eq!(s.now_playing, None, "unknown until the speaker says");
        let mut pl = vec![0, 3, 2];
        pl.extend_from_slice(b"SunMe");
        assert!(s.apply(&f(0x50, &pl)));
        let np = s.now_playing.clone().flatten().unwrap();
        assert_eq!((np.title.as_str(), np.artist.as_str()), ("Sun", "Me"));
        assert!(s.apply(&f(0x50, &[0, 0, 0, 0, 0])));
        assert_eq!(s.now_playing, Some(None), "cleared");
        assert!(!s.apply(&f(0x50, &[0, 9])), "malformed is ignored");
        s.clear_device();
        assert_eq!(s.now_playing, None);
    }

    #[test]
    fn status_line_needs_a_connection_and_a_volume() {
        let mut s = State { status: "Searching".into(), volume: Some(5), ..State::default() };
        assert_eq!(s.status_line(), "Searching", "volume alone is not enough");
        s.connected = true;
        assert_eq!(s.status_line(), "Connected - volume 5");
        s.input = Some(1);
        s.eq_preset = Some(4);
        assert_eq!(s.status_line(), "Connected - volume 5 - Bluetooth - EQ Custom");
    }

    #[test]
    fn clear_device_keeps_status_text() {
        let mut s = State::default();
        s.apply(&f(0x66, &[0x10, 7]));
        s.apply(&f(0xC6, &[2, 4, 1]));
        s.connected = true;
        s.status = "Disconnected - retrying...".into();
        s.clear_device();
        assert_eq!((s.volume, s.firmware.clone(), s.connected), (None, None, false));
        assert_eq!(s.status, "Disconnected - retrying...");
        assert_eq!(s.max_volume, 16);
    }
}
