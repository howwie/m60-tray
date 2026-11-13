//! "Start at login": a registry Run entry on Windows, a LaunchAgent on macOS.

use auto_launch::{AutoLaunch, AutoLaunchBuilder};

const APP_NAME: &str = "M60 Tray";

pub struct Startup(Option<AutoLaunch>);

impl Startup {
    pub fn new() -> Self {
        let built = std::env::current_exe().map_err(|e| e.to_string()).and_then(|exe| {
            let target = launch_target(&exe);
            AutoLaunchBuilder::new().set_app_name(APP_NAME).set_app_path(&target).build().map_err(|e| e.to_string())
        });
        match built {
            Ok(a) => Startup(Some(a)),
            Err(e) => {
                log::warn!("start-at-login unavailable: {e}");
                Startup(None)
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.0.as_ref().and_then(|a| a.is_enabled().ok()).unwrap_or(false)
    }

    pub fn set(&self, on: bool) {
        let Some(a) = &self.0 else { return };
        let r = if on { a.enable() } else { a.disable() };
        if let Err(e) = r {
            log::warn!("could not {} start at login: {e}", if on { "enable" } else { "disable" });
        }
    }
}

/// On macOS login items must point at the `.app` bundle, not the binary inside it.
fn launch_target(exe: &std::path::Path) -> String {
    exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).unwrap_or(exe).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn bundle_path_is_used_when_inside_an_app() {
        assert_eq!(
            launch_target(Path::new("/Applications/M60 Tray.app/Contents/MacOS/m60-tray")),
            "/Applications/M60 Tray.app"
        );
        assert_eq!(launch_target(Path::new("C:/Tools/m60-tray.exe")), "C:/Tools/m60-tray.exe");
    }
}
