//! The volume popup: a small frameless window shown next to the tray icon.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::busy::{self, Window};

pub const LABEL: &str = "popup";
const WIDTH: f64 = 300.0;
const HEIGHT: f64 = 214.0;
const GAP: i32 = 8;
/// A click on the tray icon makes the popup lose focus (hiding it) just before the click itself
/// arrives; without this the same click would immediately show it again.
const BLUR_GRACE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Top-left corner for a popup of `size` next to `anchor` (the tray icon), kept inside `screen`.
/// It opens above the icon when the icon is in the lower half of the screen (Windows taskbar)
/// and below it otherwise (macOS menu bar).
pub fn place(anchor: Rect, size: (i32, i32), screen: Rect, gap: i32) -> (i32, i32) {
    let x = anchor.x + anchor.w / 2 - size.0 / 2;
    let y = if anchor.y + anchor.h / 2 > screen.y + screen.h / 2 {
        anchor.y - size.1 - gap
    } else {
        anchor.y + anchor.h + gap
    };
    (
        clamp(x, screen.x + gap, screen.x + screen.w - size.0 - gap),
        clamp(y, screen.y + gap, screen.y + screen.h - size.1 - gap),
    )
}

fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    // a screen smaller than the popup would make hi < lo; prefer the top/left edge then
    v.min(hi).max(lo)
}

#[derive(Default)]
pub struct PopupState {
    hidden_by_blur: Mutex<Option<Instant>>,
}

pub fn create(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    app.manage(PopupState::default());
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("M60 volume")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .shadow(true)
        .build()?;
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            if let Some(w) = handle.get_webview_window(LABEL) {
                if w.is_visible().unwrap_or(false) {
                    if let Some(state) = handle.try_state::<PopupState>() {
                        *state.hidden_by_blur.lock().unwrap() = Some(Instant::now());
                    }
                    let _ = w.hide();
                    busy::set(&handle, Window::Popup, false);
                }
            }
        }
    });
    Ok(window)
}

pub fn hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.hide();
    }
    busy::set(app, Window::Popup, false);
}

/// Show the popup next to `anchor` (physical pixels), or hide it if it is already open.
pub fn toggle(app: &AppHandle, anchor: Option<Rect>) {
    let Some(window) = app.get_webview_window(LABEL) else { return };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        busy::set(app, Window::Popup, false);
        return;
    }
    if let Some(state) = app.try_state::<PopupState>() {
        let recent = state.hidden_by_blur.lock().unwrap().is_some_and(|t| t.elapsed() < BLUR_GRACE);
        if recent {
            return;
        }
    }
    position(app, &window, anchor);
    busy::set(app, Window::Popup, true); // sharing mode: connect now, while the popup appears
    let _ = window.show();
    let _ = window.set_focus();
}

fn position(app: &AppHandle, window: &WebviewWindow, anchor: Option<Rect>) {
    let size = window.outer_size().map(|s| (s.width as i32, s.height as i32)).unwrap_or((WIDTH as i32, HEIGHT as i32));
    let monitor = anchor
        .and_then(|a| app.monitor_from_point(f64::from(a.x), f64::from(a.y)).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let area = monitor.work_area();
    let screen = Rect { x: area.position.x, y: area.position.y, w: area.size.width as i32, h: area.size.height as i32 };
    // without an anchor (some platforms do not report the icon's position) use the bottom-right
    let anchor = anchor.unwrap_or(Rect { x: screen.x + screen.w, y: screen.y + screen.h, w: 0, h: 0 });
    let (x, y) = place(anchor, size, screen, GAP);
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0, y: 0, w: 1920, h: 1040 }; // work area above a 40 px taskbar
    const SIZE: (i32, i32) = (300, 214);

    #[test]
    fn opens_above_a_taskbar_icon_and_is_centred_on_it() {
        let icon = Rect { x: 1700, y: 1050, w: 32, h: 30 };
        // the icon is below the work area (in the taskbar), so the popup rests on the work area's bottom edge
        assert_eq!(place(icon, SIZE, SCREEN, 8), (1700 + 16 - 150, 1040 - 214 - 8));
    }

    #[test]
    fn opens_below_a_menu_bar_icon() {
        let icon = Rect { x: 900, y: 0, w: 24, h: 24 };
        assert_eq!(place(icon, SIZE, SCREEN, 8), (900 + 12 - 150, 24 + 8));
    }

    #[test]
    fn stays_on_screen_near_the_edges() {
        let right = Rect { x: 1900, y: 0, w: 20, h: 24 };
        assert_eq!(place(right, SIZE, SCREEN, 8).0, 1920 - 300 - 8);
        let left = Rect { x: 0, y: 0, w: 20, h: 24 };
        assert_eq!(place(left, SIZE, SCREEN, 8).0, 8);
        let bottom_edge = Rect { x: 960, y: 1030, w: 20, h: 10 };
        assert!(place(bottom_edge, SIZE, SCREEN, 8).1 + 214 <= 1040);
    }

    #[test]
    fn works_on_a_second_monitor_with_an_offset() {
        let screen = Rect { x: 1920, y: 0, w: 2560, h: 1400 };
        let icon = Rect { x: 4000, y: 1390, w: 32, h: 30 };
        let (x, y) = place(icon, SIZE, screen, 8);
        assert!(x >= 1920 && x + 300 <= 1920 + 2560);
        assert!(y + 214 <= 1400);
    }

    #[test]
    fn a_screen_smaller_than_the_popup_does_not_panic() {
        let tiny = Rect { x: 0, y: 0, w: 100, h: 100 };
        let _ = place(Rect { x: 50, y: 50, w: 10, h: 10 }, SIZE, tiny, 8);
    }
}
