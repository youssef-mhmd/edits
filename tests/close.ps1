# Closing the window must end the app: with no file, with a file, while the
# background index runs, and during a search. Also checks that the reading
# position is saved on close. Uses a release build and its own state folder.
#   powershell -ExecutionPolicy Bypass -File tests/close.ps1
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$exe = Join-Path $root "target\release\edits.exe"
$corpus = Join-Path $root "corpus\files\synthetic"

Add-Type @"
using System; using System.Runtime.InteropServices;
public class CloseTest { [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l); }
"@

$env:EDITS_NO_SINGLE_INSTANCE = "1"
$failed = 0

function Test-Close([string]$name, [string]$file, [int]$settleMs, [string]$port) {
  $state = Join-Path ([IO.Path]::GetTempPath()) ("edits-close-" + [guid]::NewGuid())
  New-Item -ItemType Directory $state | Out-Null
  $env:EDITS_STATE_DIR = $state
  $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = if ($port) { "--remote-debugging-port=$port" } else { "" }
  $p = if ($file) { Start-Process -FilePath $exe -ArgumentList "`"$file`"" -PassThru } else { Start-Process -FilePath $exe -PassThru }
  Start-Sleep -Milliseconds $settleMs
  if ($port) {
    # Start a search over the DevTools protocol, so it is running at close.
    node (Join-Path $PSScriptRoot "close-search.mjs") $port
  }
  $p.Refresh()
  $sent = Get-Date
  [CloseTest]::PostMessage($p.MainWindowHandle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null # WM_CLOSE, as the X button
  $exited = $p.WaitForExit(3000)
  $ms = [int]((Get-Date) - $sent).TotalMilliseconds
  $saved = $true
  if ($file) {
    $json = Join-Path $state "state.json"
    $saved = (Test-Path $json) -and ((Get-Content $json -Raw) -match [regex]::Escape((Split-Path $file -Leaf)))
  }
  if ($exited -and $saved) {
    "PASS  $name  (closed in $ms ms)"
  } else {
    "FAIL  $name  exited=$exited saved=$saved"
    $script:failed++
    if (-not $exited) { Stop-Process -Id $p.Id -Force }
  }
  Remove-Item -Recurse -Force $state -ErrorAction SilentlyContinue
}

Test-Close "no file" $null 2500 $null
Test-Close "one file" (Join-Path $corpus "latin-text-20p.pdf") 2500 $null
Test-Close "large file, index running" (Join-Path $corpus "large-1000p-500mb.pdf") 2500 $null
Test-Close "search running" (Join-Path $corpus "latin-text-2000p.pdf") 2500 "9341"
Remove-Item Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
if ($failed) { exit 1 }
