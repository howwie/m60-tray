# M60 Tray

[![CI](https://github.com/howwie/m60-tray/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/howwie/m60-tray/actions/workflows/ci.yml?query=branch%3Amain)
[![Latest release](https://img.shields.io/github/v/release/howwie/m60-tray)](https://github.com/howwie/m60-tray/releases/latest)

A small system-tray app that controls **Edifier M60** speakers from Windows and macOS, so you do not
need the phone app. It talks to the speaker over Bluetooth Low Energy using the protocol documented in
[`docs/PROTOCOL.md`](docs/PROTOCOL.md) (reverse-engineered from the official app; unofficial and not
affiliated with Edifier). The UI is built with [Tauri 2](https://tauri.app).

## Using it

- **Left-click** the tray icon: a small popup opens next to it with the volume slider (drag, scroll the
  mouse wheel, use the arrow keys or the - / + buttons) and, on the Bluetooth input, what is playing with
  media buttons. Esc or clicking elsewhere closes it.
- **Right-click** the tray icon: input, EQ preset, power off, and **Settings...** for everything else.
- **EQ preset > Edit custom presets...** opens the custom EQ editor.
- The tray icon shows the current volume, and turns grey when the speaker is not connected. On macOS it is
  a black-and-white menu bar icon that follows the light or dark menu bar (dimmed when not connected).
  While the app is connecting, three dots in the icon fill in one by one.
- The About section at the bottom of Settings shows the speaker model, its firmware version, and the app version and when it was built, so you can tell which build is running.

The speaker serves one Bluetooth control client at a time. By default the app holds that connection,
so close the phone app first. If the app says *Speaker not found*, the speaker is off, or another app
or user (the phone, another login on this PC) is connected to it. The app retries by itself.

### Share the speaker with other devices

Turn on **Share the speaker with other devices** in Settings to let the phone app or another computer use
the speaker too. The app then connects only when it needs to:

- When you hover over or click the tray icon, or open the volume popup or the EQ editor, it connects
  (the first change after a quiet spell takes a moment, roughly a second or two).
- It keeps the connection while the popup or editor is open, and for 20 seconds after your last change
  (10 seconds after closing the popup or editor), then lets go.
- While it is not connected, the menu and popup show the last known settings, and changing one connects
  first. About once a minute it connects briefly to pick up changes made elsewhere (for example with the
  phone app or the volume knob).
- If the speaker is busy with another device or switched off at that moment, the change is not made and
  the menu says the speaker is unavailable.

The setting is remembered.

### Turn off into Bluetooth

When the speaker is switched on, it starts on the input it was turned off in, and on AUX or USB it may not
be reachable over Bluetooth until something connects to it. With **Turn off into Bluetooth** on (in
Settings), powering off from the app:

1. remembers the current input and volume,
2. switches the speaker to Bluetooth and waits for it to confirm (up to 3 seconds),
3. then turns it off.

When you turn the speaker back on it starts on Bluetooth, so the app can reach it. The app keeps looking
for it (about every 10 seconds) and, once connected, switches back to the remembered input and sets the
volume. If someone has already chosen a different input by then, the app leaves it alone. This needs the
app to be running when the speaker comes back on; if it is not, the switch happens the next time it starts
and finds the speaker on Bluetooth. Turning the setting off cancels a switch-back that is still waiting.
It only applies to powering off from the app, not with the speaker's own button.

### Remember volume for each input

On Bluetooth the speaker takes its level from the connected phone or computer, so after switching back
to AUX the volume would otherwise stay at the Bluetooth level. With **Remember volume for each input**
on (the default), the app remembers the level of AUX and USB and puts it back when you switch to
them, whether you switch in the app or with the speaker's own buttons or remote. Bluetooth is left to the
connected device: forcing a level there would also change the phone's or computer's volume.

The app only does this for switches it sees happen. If it finds the speaker on a different input when it
connects (for example at a sharing-mode check), it takes the level as it is rather than making the volume
jump. A volume change you make yourself just after a switch is kept and becomes the remembered level.

## Features

- Volume (0-16): slider popup on left-click (the level shows while you hover the slider)
- Input: Bluetooth, USB, AUX (works from any input, like the phone app)
- EQ preset: Music, Monitor, Game, Movie, and your own custom presets
- Custom EQ presets: create, edit, rename and delete as many as you like (six bands, 62 Hz to 16 kHz,
  -3 to +3 dB in 0.5 dB steps), and apply one to the speaker. The speaker holds one custom profile at a
  time, like with the phone app; the one it currently holds (for example one you made in the phone app)
  is read when the app connects and added to your list.
- Settings window: the app's settings, and the speaker's Intelligent Backlight (lighting duration and
  sensitivity) and Bluetooth connection tone
- Media, in the volume popup: what the Bluetooth source is playing (title and artist), with previous,
  play, pause and next buttons. On other inputs the buttons are off, with a note that they only work on
  Bluetooth. The speaker reports the track but not whether it is playing or paused, so play and pause
  are separate buttons.
- Power off (behind a confirmation click; the speaker cannot be woken over Bluetooth afterwards), optionally
  **into Bluetooth** so the app can put your input back when the speaker is turned on again (see below)
- Shows the speaker model and firmware version (in Settings, under About)
- **Start at login** toggle, no console window, no Dock icon on macOS
- Finds the speaker by what it advertises: no addresses to configure; reconnects automatically
- Only one copy runs per login (the speaker accepts one Bluetooth control client at a time)
- **Share the speaker with other devices**: connect only when needed, so the phone app can use it too
- **Remember volume for each input**: switching to AUX or USB brings back the level that input was last
  left at (on by default)

Not included on purpose: the HD/LDAC codec switch (it makes the speaker drop Bluetooth).

## Build and install

Ready-made installers are on the [Releases](https://github.com/howwie/m60-tray/releases) page: an `.msi`
and a setup `.exe` for Windows, and a `.dmg` for Macs with Apple silicon. CI publishes a release when a
build of `main` passes with a version number that has no release yet. The installers are not signed with a
paid certificate, so Windows SmartScreen may warn on first run ("More info" > "Run anyway"), and macOS may
refuse to open the app until you allow it in System Settings > Privacy & Security ("Open Anyway").

To build it yourself, you need Rust 1.89 or newer (`rustup update`) and, for installers, the Tauri CLI:

```sh
cargo install tauri-cli --version "^2" --locked
```

### Windows 10/11

```powershell
powershell -ExecutionPolicy Bypass -File packaging\build.ps1
```

This makes an `.msi` and a setup `.exe` in `dist\`. Install either, start **M60 Tray**, then turn on
**Start at login** in its Settings. The app uses Microsoft's WebView2, which Windows 11 already has (the
installer fetches it on older systems).

For a quick build without installers: `cargo build --release -p m60-tray` gives
`target\release\m60-tray.exe`.

### macOS 11+

```sh
packaging/build.sh
```

Drag `dist/macos/M60 Tray.app` to `/Applications` (or use the `.dmg`) and open it. macOS asks for
Bluetooth permission the first time; allow it. Then turn on **Start at login** in Settings.
Run the app as the bundled `.app`: macOS stops programs that use Bluetooth without the permission text
in the bundle's `Info.plist` (`crates/m60-tray/Info.plist` is merged into it). The bundle is ad-hoc
signed, which is fine on your own Mac; for other Macs, sign with a Developer ID and notarize.

### Files the app writes

`%APPDATA%\m60-tray` (Windows) or `~/Library/Application Support/m60-tray` (macOS):
`m60-tray.log` (set `M60_LOG=debug` for more detail), `config.json` (the remembered speaker, your custom
EQ presets and the share setting) and a lock file.

## Project layout

```
crates/m60-core          protocol, device state and the connection state machine; no UI and no
                         Bluetooth backend, so it is unit-tested with a fake radio and virtual time
crates/m60-tray          the Tauri app
  src/                   btleplug backend, tray menu, popup, editor and settings windows, presets, start-at-login,
                         logging
  ui/                    the volume popup and the EQ editor (plain HTML/CSS/JS, no bundler or npm needed)
  ui-tests/              browser tests for those pages (kept out of ui/, which is bundled into the app)
  icons/                 app icons (scripts/make_icons.py regenerates them)
  capabilities/          what the popup page is allowed to call
  tauri.conf.json        Tauri configuration;  Info.plist: macOS additions
docs/PROTOCOL.md         the protocol, with the evidence for each command
packaging/               build scripts for installers
.github/workflows        CI: fmt, clippy and tests on Linux, Windows and macOS, plus installers and releases
```

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p m60-tray          # debug build, logs to the console too
```

The popup and EQ editor pages have their own tests (Node.js 18+ needed):

```sh
cd crates/m60-tray/ui-tests
npm install && npm run setup   # once
npm test
```

On Linux (development only) you need
`libdbus-1-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev pkg-config`.

The request frames in `m60-core`'s tests are byte-for-byte copies of what the official app sent in the
captures, so a protocol regression shows up as a failing test.

## Status

Tested with the real speaker on Windows and macOS: the app builds and runs on both, track changes show up
in the popup, and power off into Bluetooth (switching back to the previous input when the speaker is
turned on again) works.

Covered by automated tests: protocol frames against the captures; the connection logic (reconnect,
backoff, time-outs, quit; sharing: connect on demand, hold, release, poll; volume per input; power off
into Bluetooth and switching back) against a simulated speaker; custom EQ bytes against the capture; popup
placement; and the popup, EQ editor and settings pages (in Chromium with a stubbed Tauri API,
`crates/m60-tray/ui-tests`).

The speaker reports the same thing ("Not Provided") whether nothing is playing or a source plays without
track info (confirmed with a game on a phone), so the popup says "No track info" in both cases.
