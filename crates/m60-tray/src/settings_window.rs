//! The Settings window: app settings and the speaker's own settings.

use std::time::Duration;

use tauri::{AppHandle, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::busy::{self, Window};

pub const LABEL: &str = "settings";

/// The window's width. Its height follows the content: the page reports it with [`fit`], so
/// there is no empty space at the bottom whatever font size or text the system uses.
const WIDTH: f64 = 480.0;
/// Used until the page has measured itself (and if it never does).
const START_HEIGHT: f64 = 680.0;

/// Clamp a height the page asked for to something sensible.
fn fit_height(content: f64) -> f64 {
    if content.is_finite() {
        content.clamp(240.0, 1000.0).ceil()
    } else {
        START_HEIGHT
    }
}

/// Make the window as tall as its content. The first time, the window is still hidden: centre it
/// at its real size and show it, so it does not visibly jump.
pub fn fit(app: &AppHandle, content_height: f64) {
    let Some(w) = app.get_webview_window(LABEL) else { return };
    let height = fit_height(content_height);
    set_height(&w, height);
    log::info!("settings window: height {height} (page area now {:?})", inner_height(&w));
    if !w.is_visible().unwrap_or(true) {
        let _ = w.center();
        show(&w);
    }
}

fn set_height(w: &tauri::WebviewWindow, height: f64) {
    // the window is not resizable by the user; let the app resize it, just for this
    let _ = w.set_resizable(true);
    if let Err(e) = w.set_size(LogicalSize::new(WIDTH, height)) {
        log::warn!("settings: could not set the height to {height}: {e}");
    }
    let _ = w.set_resizable(false);
}

/// The height of the page area, in logical pixels.
fn inner_height(w: &tauri::WebviewWindow) -> Option<f64> {
    let size = w.inner_size().ok()?;
    let scale = w.scale_factor().ok()?;
    Some(size.to_logical::<f64>(scale).height)
}

fn show(w: &tauri::WebviewWindow) {
    let _ = w.show();
    let _ = w.set_focus();
}

pub fn open(app: &AppHandle) {
    busy::set(app, Window::Settings, true); // sharing mode: hold the link while it is open
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("settings.html".into()))
        .title("Settings - M60 Tray")
        .inner_size(WIDTH, START_HEIGHT)
        .resizable(false) // the page sets the height to fit its content
        .maximizable(false)
        .center()
        .visible(false) // shown by `fit` once the page has measured itself
        .build();
    match built {
        Ok(w) => {
            let handle = app.clone();
            w.on_window_event(move |event| {
                if let WindowEvent::Destroyed = event {
                    busy::set(&handle, Window::Settings, false);
                }
            });
            // in case the page never reports its size
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                if !w.is_visible().unwrap_or(true) {
                    show(&w);
                }
            });
        }
        Err(e) => {
            busy::set(app, Window::Settings, false);
            log::warn!("could not open the settings: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_height_follows_the_content_within_limits() {
        assert_eq!(fit_height(611.2), 612.0);
        assert_eq!(fit_height(10.0), 240.0);
        assert_eq!(fit_height(5000.0), 1000.0);
        assert_eq!(fit_height(f64::NAN), START_HEIGHT);
    }
}
