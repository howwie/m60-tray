//! The "Custom EQ presets" window.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::busy::{self, Window};

pub const LABEL: &str = "eq";

pub fn open(app: &AppHandle) {
    busy::set(app, Window::Editor, true); // sharing mode: hold the link while editing
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("eq.html".into()))
        .title("Custom EQ presets - M60 Tray")
        .inner_size(660.0, 420.0)
        .resizable(false) // the layout (and the sliders) has a fixed size
        .maximizable(false)
        .center()
        .focused(true)
        .build();
    match built {
        Ok(w) => {
            let handle = app.clone();
            w.on_window_event(move |event| {
                if let WindowEvent::Destroyed = event {
                    busy::set(&handle, Window::Editor, false);
                }
            });
            let _ = w.set_focus();
        }
        Err(e) => {
            busy::set(app, Window::Editor, false);
            log::warn!("could not open the EQ editor: {e}");
        }
    }
}
