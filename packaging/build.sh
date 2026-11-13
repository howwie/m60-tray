#!/usr/bin/env bash
# Builds the installable app with the Tauri bundler (run from anywhere in the repo).
#   macOS:  dist/ gets "M60 Tray.app" and a .dmg
#   Linux:  .deb / .AppImage (development only)
# Needs the Tauri CLI once:  cargo install tauri-cli --version "^2" --locked
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/crates/m60-tray"
cargo tauri build "$@"
mkdir -p "$root/dist"
cp -R "$root"/target/release/bundle/* "$root/dist/"
echo "Bundles copied to $root/dist"
