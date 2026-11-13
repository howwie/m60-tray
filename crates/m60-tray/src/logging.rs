//! A tiny file logger. The release build has no console window, so the log file
//! (`m60-tray.log` next to the config) is the only place to see what happened.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use log::{Level, LevelFilter, Log, Metadata, Record};

const MAX_LOG_BYTES: u64 = 512 * 1024;

struct FileLogger {
    file: Mutex<File>,
    also_stderr: bool,
}

impl Log for FileLogger {
    fn enabled(&self, m: &Metadata) -> bool {
        // our own crates at the configured level; dependencies only when they report a problem
        m.target().starts_with("m60") || m.level() <= Level::Warn
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!("{} {:<5} {}: {}\n", timestamp(SystemTime::now()), r.level(), r.target(), r.args());
        if let Ok(mut f) = self.file.lock() {
            let _ = f.write_all(line.as_bytes());
        }
        if self.also_stderr {
            eprint!("{line}");
        }
    }

    fn flush(&self) {}
}

/// Set up logging to `<dir>/m60-tray.log`. `M60_LOG=debug` (or trace/warn/...) changes the level.
pub fn init(dir: &Path) {
    let path = dir.join("m60-tray.log");
    if fs::metadata(&path).map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false) {
        let _ = fs::rename(&path, dir.join("m60-tray.old.log"));
    }
    let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) else { return };
    let level = std::env::var("M60_LOG").ok().and_then(|v| v.parse().ok()).unwrap_or(LevelFilter::Info);
    let logger = FileLogger { file: Mutex::new(file), also_stderr: cfg!(debug_assertions) };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(level);
    }
}

/// `YYYY-MM-DD HH:MM:SS` in UTC (no time-zone database needed for a log file).
pub fn timestamp(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // days since 1970-01-01 -> civil date (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timestamps_match_known_instants() {
        let at = |s| UNIX_EPOCH + Duration::from_secs(s);
        assert_eq!(timestamp(at(0)), "1970-01-01 00:00:00");
        assert_eq!(timestamp(at(951_782_400)), "2000-02-29 00:00:00"); // leap day
        assert_eq!(timestamp(at(1_794_355_200 + 3_661)), "2026-11-11 01:01:01");
    }
}
