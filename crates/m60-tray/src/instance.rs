//! Only one tray app per login: the speaker accepts a single BLE client, so a second copy
//! would just fight the first one for the connection.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;

/// Keep the returned file alive for as long as the app runs; the lock goes away with it.
pub fn acquire(dir: &Path) -> io::Result<Option<File>> {
    let file = OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("m60-tray.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails_until_the_first_is_dropped() {
        let d = std::env::temp_dir().join(format!("m60-test-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let first = acquire(&d).unwrap();
        assert!(first.is_some());
        assert!(acquire(&d).unwrap().is_none(), "already running");
        drop(first);
        assert!(acquire(&d).unwrap().is_some(), "free again");
        let _ = std::fs::remove_dir_all(&d);
    }
}
