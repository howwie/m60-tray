//! Tells the controller when a window that needs the speaker is open. In sharing mode the link is
//! then held (and connected straight away), and let go a little after the window closes.

use std::sync::atomic::{AtomicBool, Ordering};

use m60_core::{Command, Handle};
use tauri::{AppHandle, Manager};

#[derive(Default)]
pub struct Busy {
    popup: AtomicBool,
    editor: AtomicBool,
    settings: AtomicBool,
}

#[derive(Clone, Copy)]
pub enum Window {
    Popup,
    Editor,
    Settings,
}

pub fn set(app: &AppHandle, which: Window, open: bool) {
    let (Some(busy), Some(handle)) = (app.try_state::<Busy>(), app.try_state::<Handle>()) else { return };
    let flag = match which {
        Window::Popup => &busy.popup,
        Window::Editor => &busy.editor,
        Window::Settings => &busy.settings,
    };
    if flag.swap(open, Ordering::SeqCst) != open {
        let any = [&busy.popup, &busy.editor, &busy.settings].iter().any(|f| f.load(Ordering::SeqCst));
        handle.send(Command::SetBusy(any));
    }
}
