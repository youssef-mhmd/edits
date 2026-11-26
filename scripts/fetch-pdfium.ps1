# Downloads the pinned PDFium build into vendor/pdfium (not committed).
# Pinned to the PDFium version pdfium-render's bindings were generated for
# (feature `pdfium_8125` in Cargo.toml). Change both together.
$ErrorActionPreference = "Stop"
$Version = "8125"
$Sha256 = "9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b"
$Url = "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F$Version/pdfium-win-x64.tgz"

$root = Split-Path $PSScriptRoot -Parent
$vendor = Join-Path $root "vendor"
$dest = Join-Path $vendor "pdfium"
$stamp = Join-Path $dest "VERSION.edits"
if ((Test-Path $stamp) -and ((Get-Content $stamp) -eq $Version)) {
  Write-Host "PDFium chromium/$Version already in vendor/pdfium"
  exit 0
}

New-Item -ItemType Directory -Force $vendor | Out-Null
$tgz = Join-Path $vendor "pdfium-win-x64.tgz"
Write-Host "Downloading PDFium chromium/$Version"
Invoke-WebRequest -Uri $Url -OutFile $tgz -UseBasicParsing
$hash = (Get-FileHash $tgz -Algorithm SHA256).Hash.ToLower()
if ($hash -ne $Sha256) { throw "PDFium checksum mismatch: got $hash" }

if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
New-Item -ItemType Directory -Force $dest | Out-Null
# Windows bsdtar; a GNU tar earlier on PATH (Git) misreads "D:" as a host.
& "$env:SystemRoot\System32\tar.exe" -xzf $tgz -C $dest
if ($LASTEXITCODE -ne 0) { throw "tar failed" }
Remove-Item $tgz
Set-Content -Path $stamp -Value $Version
Write-Host "PDFium ready in vendor/pdfium"
