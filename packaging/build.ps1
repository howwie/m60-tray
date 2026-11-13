# Builds the Windows installers (.msi and setup .exe) with the Tauri bundler.
#   powershell -ExecutionPolicy Bypass -File packaging\build.ps1
# Needs the Tauri CLI once:  cargo install tauri-cli --version "^2" --locked
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Push-Location (Join-Path $root "crates\m60-tray")
try { cargo tauri build @args } finally { Pop-Location }
New-Item -ItemType Directory -Force (Join-Path $root "dist") | Out-Null
Copy-Item -Recurse -Force (Join-Path $root "target\release\bundle\*") (Join-Path $root "dist")
Write-Host "Installers copied to $root\dist"
