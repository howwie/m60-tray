# Notes for Claude

Context for working on this project in a new conversation. The README covers what the app does and
how to build it, and docs/PROTOCOL.md covers the speaker's Bluetooth protocol. This file covers
how the owner likes to work, decisions made along the way, and what has been tested on real hardware.

## How to work here

- **This folder is the source of truth.** Write changes straight into it. Don't attach changed
  files in the chat, and don't list which files changed: a short summary of what changed for the user is enough.
- **Don't write to the claude.ai Project** "edifier-m60-manager". Its copy of the code is old;
  this folder replaces it.
- **Don't touch the release profile** in the workspace Cargo.toml (`opt-level = "s"`, `lto = true`,
  `codegen-units = 1`, `strip = true`). The last build step (fat LTO of the m60-tray crate) is slow
  on purpose: the owner prefers the smallest and fastest .exe over build time.
- **Say plainly what was tested and what wasn't.** Only real hardware can confirm Bluetooth behaviour
  and how the windows look on Windows and macOS; a cloud container can't.
- When the user changes the app, bump the version in the workspace Cargo.toml (`[workspace.package]`)
  so they can tell builds apart (Settings > About shows the version and build time).
- The owner builds on Windows (`cargo build --release -p m60-tray`) and also uses macOS.

## Checks to run after a change

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                       # m60-core: fake radio + paused tokio time
cd crates/m60-tray/ui-tests && npm install && npm test   # pages in headless Chromium, Tauri API mocked
```

`SCREENSHOTS_DIR=/some/dir npm test` also saves screenshots of every page. Look at them after UI changes.

On Linux, a smoke run is `HOME=$(mktemp -d) timeout 8 xvfb-run -a ./target/debug/m60-tray`. Then check
`$HOME/.config/m60-tray/m60-tray.log`: the first line is "M60 Tray <version> (built ...) starting".

## Layout

- `crates/m60-core`: the protocol (`proto.rs`), the speaker state (`state.rs`) and the connection
  state machine (`controller.rs`, behind `Radio`/`Link` traits so tests use a fake speaker).
- `crates/m60-tray`: the Tauri 2 app.
  - `ble.rs`: the btleplug radio.
  - `tray.rs` and `menu.rs`: the tray icon and the right-click menu.
  - `icon.rs`: draws the icon in code.
  - `popup.rs`: the left-click volume and media popup.
  - `editor.rs`: the custom EQ editor window.
  - `settings_window.rs`: the Settings window.
  - `commands.rs`: what the pages can call.
  - `config.rs` and `presets.rs`: config.json.
  - `startup.rs`: start at login.
  - `instance.rs`: one copy per login.
  - `logging.rs`: the log file.
  - `version.rs`: version and build time (from `build.rs`).
- `crates/m60-tray/ui`: plain HTML/CSS/JS, no bundler. `theme.css` holds the light and dark colours.
- `crates/m60-tray/ui-tests`: Playwright tests for the three pages.
- CI: `.github/workflows/ci.yml`. When every job passes on a push to `main`, the `release` job publishes
  the Windows and macOS installers as release `v<version>` (from `[workspace.package]`). A version that
  already has a release is skipped, so bumping the version is what makes a new release. Some file-sync tools refuse paths under `.github`; if one does,
  put the workflow in `packaging/github-ci.yml` and tell the user to move it.

## Decisions (and why)

- **Right-click menu** is for everyday controls: status, Input, EQ preset (with the saved custom
  presets and "Edit custom presets..."), Power off (behind a confirmation submenu), Settings...,
  Reconnect and Quit. Volume isn't in the menu: left click opens the volume slider. Media controls
  live in the popup, not the menu.
- **Settings window** holds the app settings (start at login, share, remember volume per input,
  turn off into Bluetooth), the speaker settings (backlight duration and sensitivity, Bluetooth
  connection tone) and About (speaker model, firmware, app version, build time). It has no status
  line, because the menu already shows it. The window is 480 px wide and as tall as its content: the
  page measures itself and calls `fit_settings`. Measure without `requestAnimationFrame`, because the
  window starts hidden and a hidden window on Windows gets no animation frames. Measure with
  `main.scrollHeight` (main squeezed to height 0 while measuring), not the last element's bottom:
  the bottom margin of the About list sticks out of its section, and missing it left a scrollbar.
  The page measures again on every resize, because the width it was first laid out at can differ.
- **Popup:** media controls sit above the volume slider. The slider shows the level only in a tooltip
  while hovering or dragging, with no "7/16" text. Media buttons only work on the Bluetooth input; on
  other inputs they're disabled, with a note. Track info comes only when the speaker pushes it.
  "Not Provided" covers both nothing playing and something playing with no track info (a phone game,
  confirmed), so the popup says "No track info".
- **Share the speaker** (the speaker takes one Bluetooth control client at a time): connect on
  demand, let go 20 s after the last use (10 s after a window closes), and check every 60 s (every
  10 s while waiting for the speaker to come back on). Changes made while released are queued, and
  volume changes are merged. Pointing at the tray icon starts connecting early.
- **Remember volume for each input:** only AUX and USB. Bluetooth volume follows the phone.
  It's restored when the input changes, from the app or on the speaker. The app waits 2 s for the
  level to settle and tries once more if needed. A change the user makes during that time wins.
- **Turn off into Bluetooth** (the speaker powers on into its last input, and only Bluetooth lets
  the app reach it):
  1. On power off, the app remembers the input and volume (saved in config.json as `resume`).
  2. It switches to Bluetooth, waits up to 3 s for the speaker to confirm, then turns the speaker off.
  3. When the speaker comes back on Bluetooth, the app puts the input and volume back.
  4. If the speaker comes back on some other input, the app leaves it alone.
  5. Turning the setting off cancels a pending restore.
- **Tray icon:** a rounded square with the volume.
  - Windows: blue when connected, grey when not.
  - macOS: a template image (black, alpha only), as menu bar icons should be.
  - While connecting, three dots fill in (350 ms per frame).
  - Don't use Edifier's logo or other branding in the icon.

## Tested on real hardware (by the owner)

- Windows and macOS builds run. The macOS icon matches the menu bar.
- Track changes reach the popup while it's open.
- Power off into Bluetooth works, including the restore when the speaker comes back on.
- A Bluetooth source that isn't playing, and a phone game playing, both show no track info.
- The Settings window fits its content with no scrollbar on Windows (0.9.2). 0.9.1 under-measured
  by the About list's 4 px bottom margin; the page tests now check the page at exactly the height it asks for.
