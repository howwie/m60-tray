//! The app's own list of custom EQ presets, saved in config.json together with the remembered
//! speaker. The speaker only stores one custom profile at a time; applying a preset copies it there.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use m60_core::proto::{self, CustomEq};
use m60_core::Resume;

use crate::config::{self, Config, Preset, ResumeConfig};

pub struct Store {
    dir: PathBuf,
    cfg: Mutex<Config>,
}

pub fn now_secs() -> u32 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0)
}

impl Store {
    pub fn open(dir: &Path) -> Store {
        Store { dir: dir.to_path_buf(), cfg: Mutex::new(config::load(dir)) }
    }

    fn change<T>(&self, f: impl FnOnce(&mut Config) -> T) -> T {
        let mut cfg = self.cfg.lock().unwrap();
        let out = f(&mut cfg);
        config::save(&self.dir, &cfg);
        out
    }

    pub fn address(&self) -> Option<String> {
        self.cfg.lock().unwrap().address.clone()
    }

    pub fn set_address(&self, id: &str) {
        if self.address().as_deref() != Some(id) {
            self.change(|c| c.address = Some(id.to_string()));
        }
    }

    pub fn sharing(&self) -> bool {
        self.cfg.lock().unwrap().share
    }

    pub fn set_sharing(&self, on: bool) {
        self.change(|c| c.share = on);
    }

    /// "Remember volume for each input" (on unless the user turned it off).
    pub fn remember_volume(&self) -> bool {
        self.cfg.lock().unwrap().remember_volume.unwrap_or(true)
    }

    pub fn set_remember_volume(&self, on: bool) {
        self.change(|c| c.remember_volume = Some(on));
    }

    pub fn bluetooth_on_power_off(&self) -> bool {
        self.cfg.lock().unwrap().bluetooth_on_power_off
    }

    pub fn set_bluetooth_on_power_off(&self, on: bool) {
        self.change(|c| c.bluetooth_on_power_off = on);
    }

    pub fn resume(&self) -> Option<Resume> {
        self.cfg.lock().unwrap().resume.map(|r| Resume { input: r.input, volume: r.volume })
    }

    pub fn set_resume(&self, resume: Option<Resume>) {
        let r = resume.map(|r| ResumeConfig { input: r.input, volume: r.volume });
        if self.cfg.lock().unwrap().resume != r {
            self.change(|c| c.resume = r);
        }
    }

    pub fn input_volumes(&self) -> HashMap<u8, u8> {
        self.cfg.lock().unwrap().input_volumes.iter().map(|(k, v)| (*k, *v)).collect()
    }

    pub fn set_input_volumes(&self, levels: &HashMap<u8, u8>) {
        let levels = levels.iter().map(|(k, v)| (*k, *v)).collect();
        if self.cfg.lock().unwrap().input_volumes != levels {
            self.change(|c| c.input_volumes = levels);
        }
    }

    pub fn presets(&self) -> Vec<Preset> {
        self.cfg.lock().unwrap().presets.clone()
    }

    pub fn get(&self, id: u32) -> Option<Preset> {
        self.cfg.lock().unwrap().presets.iter().find(|p| p.id == id).cloned()
    }

    /// Create (`id == None`) or update a preset. Gains are snapped to 0.5 dB steps in -3..+3 dB.
    pub fn save(&self, id: Option<u32>, name: &str, gains: [f32; 6], now: u32) -> Result<Preset, String> {
        let name = proto::clean_profile_name(name);
        if name.is_empty() {
            return Err("Give the preset a name.".into());
        }
        let gains = gains.map(proto::snap_gain);
        let mut cfg = self.cfg.lock().unwrap();
        if cfg.presets.iter().any(|p| Some(p.id) != id && p.name.eq_ignore_ascii_case(&name)) {
            return Err(format!("There is already a preset called \"{name}\"."));
        }
        let preset = match id {
            Some(id) => {
                let p = cfg.presets.iter_mut().find(|p| p.id == id).ok_or("That preset no longer exists.")?;
                p.name = name;
                p.gains = gains;
                p.clone()
            }
            None => {
                // the id doubles as the creation time; keep it unique even for quick successive saves
                let id = cfg.presets.iter().map(|p| p.id.saturating_add(1)).chain([now]).max().unwrap_or(now);
                let p = Preset { id, name, gains };
                cfg.presets.push(p.clone());
                p
            }
        };
        config::save(&self.dir, &cfg);
        Ok(preset)
    }

    pub fn delete(&self, id: u32) -> bool {
        self.change(|c| {
            let before = c.presets.len();
            c.presets.retain(|p| p.id != id);
            if !c.deleted_presets.contains(&id) {
                c.deleted_presets.push(id);
            }
            c.presets.len() != before
        })
    }

    /// Add the speaker's own custom profile (e.g. one made in the phone app) to the list, unless we
    /// already have it or the user deleted it. Returns true if the list changed.
    pub fn import(&self, eq: &CustomEq) -> bool {
        let id = eq.id();
        let name = proto::clean_profile_name(&eq.name);
        {
            let cfg = self.cfg.lock().unwrap();
            if name.is_empty() || cfg.presets.iter().any(|p| p.id == id) || cfg.deleted_presets.contains(&id) {
                return false;
            }
        }
        let gains: Vec<f32> = eq.gains_db();
        let Ok(gains) = <[f32; 6]>::try_from(gains) else { return false };
        self.change(|c| {
            // a name clash with a different preset gets a suffix rather than being refused
            let mut unique = name.clone();
            let mut n = 2;
            while c.presets.iter().any(|p| p.name.eq_ignore_ascii_case(&unique)) {
                unique = proto::clean_profile_name(&format!("{name} ({n})"));
                n += 1;
            }
            c.presets.push(Preset { id, name: unique, gains: gains.map(proto::snap_gain) });
        });
        true
    }
}

/// The profile to send for a saved preset.
pub fn to_profile(p: &Preset) -> CustomEq {
    CustomEq::new(&p.name, p.gains, p.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (Store, PathBuf) {
        let d = std::env::temp_dir().join(format!("m60-test-presets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        (Store::open(&d), d)
    }

    #[test]
    fn create_update_delete_and_persist() {
        let (s, d) = store("crud");
        let a = s.save(None, "  Late night ", [1.26, -9.0, 0.0, 0.0, 0.0, 3.0], 1000).unwrap();
        assert_eq!((a.id, a.name.as_str()), (1000, "Late night"));
        assert_eq!(a.gains, [1.5, -3.0, 0.0, 0.0, 0.0, 3.0], "snapped and clamped");
        let b = s.save(None, "Podcast", [0.0; 6], 1000).unwrap();
        assert_eq!(b.id, 1001, "unique even within the same second");
        let a2 = s.save(Some(a.id), "Night", [0.5; 6], 2000).unwrap();
        assert_eq!((a2.id, a2.name.as_str()), (1000, "Night"), "updating keeps the id");

        // a fresh store reads the same file
        let again = Store::open(&d);
        assert_eq!(again.presets().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Night", "Podcast"]);

        assert!(s.delete(b.id));
        assert!(!s.delete(b.id), "already gone");
        assert_eq!(Store::open(&d).presets().len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rejects_bad_input() {
        let (s, d) = store("bad");
        assert!(s.save(None, "   ", [0.0; 6], 1).is_err(), "no name");
        s.save(None, "Music", [0.0; 6], 1).unwrap();
        assert!(s.save(None, "music", [0.0; 6], 2).is_err(), "duplicate name, any case");
        assert!(s.save(Some(999), "Other", [0.0; 6], 3).is_err(), "unknown id");
        let p = s.presets()[0].clone();
        assert!(s.save(Some(p.id), "MUSIC", [1.0; 6], 4).is_ok(), "renaming a preset to its own name is fine");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn imports_the_speakers_profile_once_and_respects_deletion() {
        let (s, d) = store("import");
        let gaming = CustomEq::new("Gaming", [-3.0, -3.0, 0.0, 0.0, 0.0, 0.0], 1_794_308_922);
        assert!(s.import(&gaming));
        assert!(!s.import(&gaming), "already there");
        let p = s.get(1_794_308_922).unwrap();
        assert_eq!((p.name.as_str(), p.gains[0]), ("Gaming", -3.0));
        assert_eq!(to_profile(&p), gaming, "round-trips to the same profile");

        s.delete(p.id);
        assert!(!s.import(&gaming), "deleted stays deleted");

        s.save(None, "Music", [0.0; 6], 5).unwrap();
        let other = CustomEq::new("Music", [1.0; 6], 77);
        assert!(s.import(&other));
        assert_eq!(s.get(77).unwrap().name, "Music (2)", "name clash gets a suffix");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn address_is_kept_alongside_presets() {
        let (s, d) = store("addr");
        s.save(None, "A", [0.0; 6], 1).unwrap();
        s.set_address("C8:24:70:AA:49:66");
        s.set_sharing(true);
        let again = Store::open(&d);
        assert_eq!((again.address().as_deref(), again.presets().len()), (Some("C8:24:70:AA:49:66"), 1));
        assert!(again.sharing(), "the share setting is kept too");
        assert!(again.remember_volume(), "volume memory is on unless turned off");
        s.set_remember_volume(false);
        s.set_input_volumes(&HashMap::from([(3, 6), (2, 9)]));
        let again = Store::open(&d);
        assert!(!again.remember_volume());
        assert_eq!(again.input_volumes(), HashMap::from([(3, 6), (2, 9)]));
        assert!(!again.bluetooth_on_power_off(), "power off into Bluetooth is off unless turned on");
        s.set_bluetooth_on_power_off(true);
        s.set_resume(Some(Resume { input: 3, volume: Some(9) }));
        let again = Store::open(&d);
        assert!(again.bluetooth_on_power_off());
        assert_eq!(again.resume(), Some(Resume { input: 3, volume: Some(9) }));
        s.set_resume(None);
        assert_eq!(Store::open(&d).resume(), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
