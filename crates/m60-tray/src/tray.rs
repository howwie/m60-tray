//! Tray icon, its menu, and the plumbing that keeps them and the popup in step with the speaker.

use std::sync::Mutex;
use std::time::Duration;

use std::sync::Arc;

use m60_core::{Command, Handle, State};
use tauri::image::Image;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::{PresetsDto, SettingsDto, StateDto};
use crate::menu::{parse_id, Action, TrayMenu};
use crate::popup::{self, Rect};
use crate::presets::{self, Store};
use crate::{busy, editor, icon, settings_window, startup::Startup};

const TRAY_ID: &str = "main";

struct Shared {
    menu: Mutex<TrayMenu>,
    startup: Startup,
    shown_icon: Mutex<Option<IconKey>>,
}

/// What the tray icon currently shows.
#[derive(Debug, Clone, Copy, PartialEq)]
enum IconKey {
    Volume(Option<u8>, bool),
    /// Connecting animation frame.
    Connecting(u8),
}

/// How fast the connecting dots fill in.
const CONNECTING_FRAME: Duration = Duration::from_millis(350);

/// On macOS the icon is a template image: macOS draws it black or white to suit the menu bar.
const ICON_IS_TEMPLATE: bool = matches!(icon::PLATFORM_STYLE, icon::Style::Template);

fn make_icon(key: IconKey) -> Image<'static> {
    let px = match key {
        IconKey::Volume(volume, connected) => icon::render_styled(volume, connected, icon::PLATFORM_STYLE),
        IconKey::Connecting(frame) => icon::render_connecting(frame, icon::PLATFORM_STYLE),
    };
    Image::new_owned(px, icon::SIZE, icon::SIZE)
}

/// Show `key` unless it is already showing. Call with `shared.shown_icon` locked.
fn show_icon(app: &AppHandle, last: &mut Option<IconKey>, key: IconKey) {
    if *last == Some(key) {
        return;
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(Some(make_icon(key)));
        if ICON_IS_TEMPLATE {
            let _ = tray.set_icon_as_template(true); // keep it a template after replacing the image
        }
        *last = Some(key);
    }
}

pub fn install(app: &AppHandle, handle: Handle) -> tauri::Result<()> {
    popup::create(app)?;
    let mut menu = TrayMenu::new(app)?;
    if let Some(store) = app.try_state::<Arc<Store>>() {
        menu.set_presets(app, &store.presets())?;
    }
    let tray_menu = menu.menu.clone();
    app.manage(Shared {
        menu: Mutex::new(menu),
        startup: Startup::new(),
        shown_icon: Mutex::new(Some(IconKey::Volume(None, false))),
    });
    app.manage(busy::Busy::default());

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(make_icon(IconKey::Volume(None, false)))
        .icon_as_template(ICON_IS_TEMPLATE)
        .tooltip("Edifier M60")
        .menu(&tray_menu)
        .show_menu_on_left_click(false) // left click = volume slider, right click = menu
        .on_tray_icon_event(|tray, event| {
            // Sharing mode: the pointer on the icon means a click is coming, so connect now.
            if matches!(event, TrayIconEvent::Enter { .. } | TrayIconEvent::Click { button: MouseButton::Right, .. }) {
                if let Some(h) = tray.app_handle().try_state::<Handle>() {
                    if h.state().sharing {
                        h.send(Command::Wake);
                    }
                }
            }
            if let TrayIconEvent::Click {
                button: MouseButton::Left, button_state: MouseButtonState::Up, rect, ..
            } = event
            {
                let scale = tray.app_handle().primary_monitor().ok().flatten().map_or(1.0, |m| m.scale_factor());
                let pos = rect.position.to_physical::<i32>(scale);
                let size = rect.size.to_physical::<i32>(scale);
                popup::toggle(tray.app_handle(), Some(Rect { x: pos.x, y: pos.y, w: size.width, h: size.height }));
            }
        })
        .on_menu_event({
            let handle = handle.clone();
            move |app, event| on_menu(app, &handle, event.id().as_ref())
        })
        .build(app)?;

    // While connecting, the dots in the icon fill in one by one.
    let (app3, anim_handle) = (app.clone(), handle.clone());
    tauri::async_runtime::spawn(async move {
        let mut frame = 0u8;
        loop {
            tokio::time::sleep(CONNECTING_FRAME).await;
            let Some(shared) = app3.try_state::<Shared>() else { continue };
            let mut last = shared.shown_icon.lock().unwrap();
            // checked under the lock, so a finished connection's icon is never painted over
            if anim_handle.state().connecting {
                frame = (frame + 1) % icon::CONNECTING_FRAMES;
                show_icon(&app3, &mut last, IconKey::Connecting(frame));
            } else {
                frame = 0;
            }
        }
    });

    // Push every state change to the menu, the icon and the popup.
    let app2 = app.clone();
    let mut rx = handle.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let state = rx.borrow_and_update().clone();
            apply_state(&app2, &state, false);
            if rx.changed().await.is_err() {
                break;
            }
        }
    });
    Ok(())
}

fn apply_state(app: &AppHandle, state: &State, force: bool) {
    let Some(shared) = app.try_state::<Shared>() else { return };
    // A custom profile on the speaker that we have not seen (e.g. made in the phone app) joins the list.
    if let (Some(eq), Some(store)) = (&state.custom_eq, app.try_state::<Arc<Store>>()) {
        if store.import(eq) {
            log::info!("imported custom EQ \"{}\" from the speaker", eq.name);
            let _ = shared.menu.lock().unwrap().set_presets(app, &store.presets());
        }
    }
    shared.menu.lock().unwrap().apply(state, force);

    {
        let mut last = shared.shown_icon.lock().unwrap();
        if !state.connecting {
            show_icon(app, &mut last, IconKey::Volume(state.volume, state.connected));
        } else if !matches!(*last, Some(IconKey::Connecting(_))) {
            show_icon(app, &mut last, IconKey::Connecting(0)); // the animation takes it from here
        }
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(format!("Edifier M60 - {}", state.status_line())));
    }
    let _ = app.emit("state", StateDto::from(state));
    if let Some(store) = app.try_state::<Arc<Store>>() {
        let _ = app.emit("presets", PresetsDto::new(&store, state));
    }
    let _ = app.emit("settings", SettingsDto::new(shared.startup.is_enabled(), state));
}

pub fn start_at_login(app: &AppHandle) -> bool {
    app.try_state::<Shared>().is_some_and(|s| s.startup.is_enabled())
}

pub fn set_start_at_login(app: &AppHandle, on: bool) {
    if let Some(s) = app.try_state::<Shared>() {
        s.startup.set(on);
    }
}

/// Re-send everything to the menu and the windows (after a setting that is not speaker state changed).
pub fn refresh(app: &AppHandle) {
    if let Some(handle) = app.try_state::<Handle>() {
        apply_state(app, &handle.state(), true);
    }
}

/// Call after the saved presets changed: refreshes the menu and the editor window.
pub fn presets_changed(app: &AppHandle, handle: &Handle) {
    if let (Some(shared), Some(store)) = (app.try_state::<Shared>(), app.try_state::<Arc<Store>>()) {
        if let Err(e) = shared.menu.lock().unwrap().set_presets(app, &store.presets()) {
            log::warn!("menu: {e}");
        }
    }
    apply_state(app, &handle.state(), true);
}

fn on_menu(app: &AppHandle, handle: &Handle, id: &str) {
    match parse_id(id) {
        Some(Action::Send(cmd)) => handle.send(cmd),
        Some(Action::ApplyPreset(id)) => {
            if let Some(p) = app.try_state::<Arc<Store>>().and_then(|s| s.get(id)) {
                handle.send(Command::ApplyCustomEq(presets::to_profile(&p)));
            }
        }
        Some(Action::OpenEqEditor) => editor::open(app),
        Some(Action::OpenSettings) => settings_window::open(app),
        Some(Action::Quit) => {
            let (app, handle) = (app.clone(), handle.clone());
            tauri::async_runtime::spawn(async move {
                // Ask the controller to close the Bluetooth link, but never hang on exit.
                handle.send(Command::Quit);
                let mut rx = handle.subscribe();
                let _ = tokio::time::timeout(Duration::from_millis(1500), rx.wait_for(|s| s.status == "Stopped")).await;
                app.exit(0);
            });
            return;
        }
        None => {}
    }
    // Undo the tick the OS toggled by itself on click; the real state follows in a moment.
    apply_state(app, &handle.state(), true);
}
