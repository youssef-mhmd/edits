# Builds the signed Windows installer and its updater signature.
#
#   ./scripts/build-installer.ps1 [-Key PATH]
#
# The updater key (default: ~/.tauri/edits-updater.key) signs the installer
# so that installed copies accept it as an update. It is never stored in
# the repository. Output: target/release/bundle/nsis/edits_<version>_x64-setup.exe
# and its .sig file.

param(
  [string]$Key = (Join-Path $HOME ".tauri\edits-updater.key")
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path $Key)) {
  throw "Updater key not found at $Key. Create one with: npx tauri signer generate -w `"$Key`""
}
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw $Key
if (-not $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) { $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "" }

npx tauri build
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

$out = Join-Path $PSScriptRoot "..\target\release\bundle\nsis"
Get-ChildItem $out -Filter "*setup.exe*" | ForEach-Object {
  "{0}  {1:N1} MB" -f $_.Name, ($_.Length / 1MB)
}
