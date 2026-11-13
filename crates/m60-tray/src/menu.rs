//! The tray's right-click menu: the everyday controls. Settings (and the speaker's firmware and the
//! app's version) live in the Settings window.
//! Items are created once; [`TrayMenu::apply`] updates their text, ticks and enabled state from
//! the speaker's [`State`].

use m60_core::{proto, Command, State};

use crate::config::Preset;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Wry};

/// What a click should do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Send(Command),
    /// Put this saved custom preset on the speaker.
    ApplyPreset(u32),
    OpenEqEditor,
    OpenSettings,
    Quit,
}

const ID_QUIT: &str = "quit";
const ID_RECONNECT: &str = "reconnect";
const ID_POWER_OFF: &str = "power-off";
const ID_EQ_EDITOR: &str = "eq-editor";
const ID_SETTINGS: &str = "settings";

/// Menu item ids carry their meaning: `input:2`, `eq:1`, and `custom:<preset id>` for saved
/// custom EQ presets. Volume is not in the menu: left-clicking the tray icon opens the volume slider.
pub fn parse_id(id: &str) -> Option<Action> {
    let send = |c| Some(Action::Send(c));
    match id {
        ID_QUIT => Some(Action::Quit),
        ID_RECONNECT => send(Command::Reconnect),
        ID_POWER_OFF => send(Command::PowerOff),
        ID_EQ_EDITOR => Some(Action::OpenEqEditor),
        ID_SETTINGS => Some(Action::OpenSettings),
        _ => {
            let (kind, value) = id.split_once(':')?;
            if kind == "custom" {
                return value.parse().ok().map(Action::ApplyPreset);
            }
            let n: u8 = value.parse().ok()?;
            match kind {
                "input" => send(Command::SetInput(n)),
                "eq" => send(Command::SetEq(n)),
                _ => None,
            }
        }
    }
}

type Radio = Vec<(u8, CheckMenuItem<Wry>)>;

pub struct TrayMenu {
    pub menu: Menu<Wry>,
    status: MenuItem<Wry>,
    input_sub: Submenu<Wry>,
    inputs: Radio,
    eq_sub: Submenu<Wry>,
    eqs: Radio,
    /// Saved custom presets, shown in the EQ submenu after the built-in ones.
    customs: Vec<(u32, String, CheckMenuItem<Wry>)>,
    power_sub: Submenu<Wry>,
    power_yes: MenuItem<Wry>,
    last: Option<State>,
}

fn radio(app: &AppHandle, prefix: &str, table: &[(u8, &str)]) -> tauri::Result<Radio> {
    table
        .iter()
        .map(|(key, name)| {
            Ok((*key, CheckMenuItem::with_id(app, format!("{prefix}:{key}"), *name, false, false, None::<&str>)?))
        })
        .collect()
}

fn submenu(app: &AppHandle, title: &str, items: &Radio) -> tauri::Result<Submenu<Wry>> {
    let list: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|(_, i)| i as &dyn IsMenuItem<Wry>).collect();
    Submenu::with_items(app, title, false, &list)
}

/// The confirmation item's text: says where the speaker will come back on.
fn power_text(st: &State) -> &'static str {
    if st.bluetooth_on_power_off && st.input.is_some_and(|i| i != proto::INPUT_BLUETOOTH) {
        "Yes, turn the speaker off now (into Bluetooth)"
    } else {
        "Yes, turn the speaker off now"
    }
}

impl TrayMenu {
    pub fn new(app: &AppHandle) -> tauri::Result<Self> {
        let item = |id: &str, text: &str, enabled: bool| MenuItem::with_id(app, id, text, enabled, None::<&str>);

        let status = item("status", "Starting...", false)?;

        let inputs = radio(app, "input", proto::INPUTS)?;
        let input_sub = submenu(app, "Input", &inputs)?;
        // Built-in presets only: "Custom" is replaced by the named custom presets below them.
        let builtin: Vec<(u8, &str)> =
            proto::EQ_PRESETS.iter().copied().filter(|(k, _)| *k != proto::EQ_CUSTOM).collect();
        let eqs = radio(app, "eq", &builtin)?;
        let eq_sub = submenu(app, "EQ preset", &eqs)?;
        eq_sub.append(&PredefinedMenuItem::separator(app)?)?;
        eq_sub.append(&item(ID_EQ_EDITOR, "Edit custom presets...", true)?)?;

        // the second click is the confirmation: turning the speaker off cannot be undone over Bluetooth
        let power_yes = item(ID_POWER_OFF, "Yes, turn the speaker off now", true)?;
        let power_sub = Submenu::with_items(app, "Power off speaker...", false, &[&power_yes])?;

        let settings = item(ID_SETTINGS, "Settings...", true)?;
        let reconnect = item(ID_RECONNECT, "Reconnect", true)?;
        let quit = item(ID_QUIT, "Quit", true)?;

        let sep = || PredefinedMenuItem::separator(app);
        let menu = Menu::with_items(
            app,
            &[
                &status,
                &sep()?,
                &input_sub,
                &eq_sub,
                &sep()?,
                &power_sub,
                &sep()?,
                &settings,
                &reconnect,
                &sep()?,
                &quit,
            ],
        )?;

        Ok(TrayMenu {
            menu,
            status,
            input_sub,
            inputs,
            eq_sub,
            eqs,
            customs: Vec::new(),
            power_sub,
            power_yes,
            last: None,
        })
    }

    /// Show these saved presets in the EQ submenu (between the built-ins and "Edit custom presets...").
    pub fn set_presets(&mut self, app: &AppHandle, presets: &[Preset]) -> tauri::Result<()> {
        let same = presets.len() == self.customs.len()
            && presets.iter().zip(&self.customs).all(|(p, (id, name, _))| p.id == *id && &p.name == name);
        if same {
            return Ok(());
        }
        for (_, _, item) in self.customs.drain(..) {
            self.eq_sub.remove(&item)?;
        }
        let first = self.eqs.len() + 1; // after the built-ins and their separator
        for (i, p) in presets.iter().enumerate() {
            let item = CheckMenuItem::with_id(app, format!("custom:{}", p.id), &p.name, false, false, None::<&str>)?;
            self.eq_sub.insert(&item, first + i)?;
            self.customs.push((p.id, p.name.clone(), item));
        }
        self.last = None; // ticks and enabled state must be redone for the new items
        Ok(())
    }

    /// Bring every item in line with `st`. Cheap to call often: it does nothing if the state did
    /// not change. `force` redoes everything (the OS flips a ticked item by itself when it is
    /// clicked, which we must undo if the state did not change).
    pub fn apply(&mut self, st: &State, force: bool) {
        if !force && self.last.as_ref() == Some(st) {
            return;
        }
        // Connected, or (sharing) released with known settings: a change then connects first.
        let on = st.controllable();
        let _ = self.status.set_text(st.status_line());

        for s in [&self.input_sub, &self.eq_sub, &self.power_sub] {
            let _ = s.set_enabled(on);
        }
        for group in [&self.inputs, &self.eqs] {
            for (_, item) in group {
                let _ = item.set_enabled(on);
            }
        }
        tick(&self.inputs, st.input);
        tick(&self.eqs, st.eq_preset);
        let active = match (st.eq_preset, &st.custom_eq) {
            (Some(proto::EQ_CUSTOM), Some(c)) => Some(c.id()),
            _ => None,
        };
        for (id, _, item) in &self.customs {
            let _ = item.set_enabled(on);
            let _ = item.set_checked(active == Some(*id));
        }
        let _ = self.power_yes.set_text(power_text(st));
        self.last = Some(st.clone());
    }
}

fn tick(group: &Radio, current: Option<u8>) {
    for (k, item) in group {
        let _ = item.set_checked(current == Some(*k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(c: Command) -> Option<Action> {
        Some(Action::Send(c))
    }

    #[test]
    fn ids_map_to_actions() {
        assert_eq!(parse_id("input:2"), send(Command::SetInput(2)));
        assert_eq!(parse_id("eq:4"), send(Command::SetEq(4)));
        assert_eq!(parse_id("power-off"), send(Command::PowerOff));
        assert_eq!(parse_id("reconnect"), send(Command::Reconnect));
        assert_eq!(parse_id("settings"), Some(Action::OpenSettings));
        assert_eq!(parse_id("quit"), Some(Action::Quit));
        assert_eq!(parse_id("custom:1794308922"), Some(Action::ApplyPreset(1_794_308_922)));
        assert_eq!(parse_id("eq-editor"), Some(Action::OpenEqEditor));
    }

    #[test]
    fn unknown_or_malformed_ids_do_nothing() {
        for id in [
            "",
            "status",
            "device",
            "version",
            "input:",
            "input:x",
            "input:300",
            "eq:-1",
            "nope:1",
            ":1",
            "vol:7",
            "custom:",
            "custom:x",
            "custom:-5",
            "custom:99999999999",
            "media:4",
            // settings moved to the Settings window
            "dur:1",
            "sens:0",
            "prompt",
            "startup",
            "share",
            "volume-memory",
        ] {
            assert_eq!(parse_id(id), None, "{id:?}");
        }
    }

    #[test]
    fn every_id_the_menu_builds_is_understood() {
        let mut ids: Vec<String> = Vec::new();
        for (prefix, table) in [("input", proto::INPUTS), ("eq", proto::EQ_PRESETS)] {
            ids.extend(table.iter().map(|(k, _)| format!("{prefix}:{k}")));
        }
        for id in ids {
            assert!(parse_id(&id).is_some(), "{id}");
        }
    }

    #[test]
    fn the_power_off_item_says_when_it_goes_into_bluetooth() {
        let mut st = State { input: Some(3), ..State::default() };
        assert_eq!(power_text(&st), "Yes, turn the speaker off now");
        st.bluetooth_on_power_off = true;
        assert_eq!(power_text(&st), "Yes, turn the speaker off now (into Bluetooth)");
        st.input = Some(1);
        assert_eq!(power_text(&st), "Yes, turn the speaker off now", "already on Bluetooth");
    }
}
