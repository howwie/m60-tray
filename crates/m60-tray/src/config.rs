//! Where the app keeps its small files: settings (remembered speaker, EQ presets), log, instance lock.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const APP_DIR: &str = "m60-tray";

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Platform id of the speaker we last connected to (tried first on the next start).
    #[serde(default)]
    pub address: Option<String>,
    /// Custom EQ presets, in the order they are listed.
    #[serde(default)]
    pub presets: Vec<Preset>,
    /// Ids the user deleted, so a profile still stored on the speaker is not imported again.
    #[serde(default)]
    pub deleted_presets: Vec<u32>,
    /// "Share the speaker with other devices": connect only when needed.
    #[serde(default)]
    pub share: bool,
    /// "Remember volume for each input" (unset means on).
    #[serde(default)]
    pub remember_volume: Option<bool>,
    /// The remembered level for each input (input id -> volume); Bluetooth is never stored.
    #[serde(default)]
    pub input_volumes: BTreeMap<u8, u8>,
    /// "Turn off into Bluetooth": power off from the app switches to Bluetooth first.
    #[serde(default)]
    pub bluetooth_on_power_off: bool,
    /// After such a power off: the input (and volume) to put back when the speaker is next on.
    #[serde(default)]
    pub resume: Option<ResumeConfig>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResumeConfig {
    pub input: u8,
    pub volume: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Preset {
    /// Creation time (Unix seconds); also sent to the speaker as the profile id, as the phone app does.
    pub id: u32,
    pub name: String,
    /// dB for 62 Hz, 250 Hz, 1 kHz, 4 kHz, 8 kHz, 16 kHz.
    pub gains: [f32; 6],
}

/// `%APPDATA%\m60-tray` on Windows, `~/Library/Application Support/m60-tray` on macOS.
pub fn app_dir() -> PathBuf {
    let dir = dirs::config_dir().unwrap_or_else(std::env::temp_dir).join(APP_DIR);
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn load(dir: &Path) -> Config {
    fs::read_to_string(dir.join("config.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(dir: &Path, cfg: &Config) {
    match serde_json::to_string_pretty(cfg) {
        Ok(s) => {
            if let Err(e) = fs::write(dir.join("config.json"), s) {
                log::warn!("could not save config: {e}");
            }
        }
        Err(e) => log::warn!("could not encode config: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("m60-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn roundtrip_and_defaults() {
        let d = temp("config");
        assert_eq!(load(&d), Config::default(), "missing file");
        save(&d, &Config { address: Some("C8:24:70:AA:49:66".into()), ..Config::default() });
        assert_eq!(load(&d).address.as_deref(), Some("C8:24:70:AA:49:66"));
        fs::write(d.join("config.json"), "{ not json").unwrap();
        assert_eq!(load(&d), Config::default(), "corrupt file is ignored");
        fs::write(d.join("config.json"), "{}").unwrap();
        assert_eq!(load(&d), Config::default(), "unknown/missing fields");
        let mut c = Config {
            share: true,
            remember_volume: Some(false),
            bluetooth_on_power_off: true,
            resume: Some(ResumeConfig { input: 3, volume: Some(9) }),
            ..Config::default()
        };
        c.input_volumes.insert(3, 6);
        save(&d, &c);
        assert_eq!(load(&d), c, "share, volume memory and levels round-trip");
        fs::write(d.join("config.json"), r#"{"address":"X"}"#).unwrap();
        assert_eq!(load(&d).address.as_deref(), Some("X"), "a config from an older version still loads");
        let _ = fs::remove_dir_all(&d);
    }
}
