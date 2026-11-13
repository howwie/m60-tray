//! The app's version and build time, shown in the Settings window and written to the log.

/// e.g. "0.9.0"
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// e.g. "2026-11-13 12:57 UTC" (set by build.rs each time the app is rebuilt)
pub const BUILD_TIME: &str = env!("M60_BUILD_TIME");
/// e.g. "M60 Tray 0.9.0 (built 2026-11-13 12:57 UTC)"
pub const VERSION_LINE: &str = concat!("M60 Tray ", env!("CARGO_PKG_VERSION"), " (built ", env!("M60_BUILD_TIME"), ")");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_names_the_version_and_build_time() {
        assert!(VERSION_LINE.starts_with(&format!("M60 Tray {APP_VERSION} (built ")));
        assert!(BUILD_TIME.ends_with(" UTC"), "{BUILD_TIME}");
        assert!(VERSION_LINE.ends_with(&format!("{BUILD_TIME})")));
    }
}
