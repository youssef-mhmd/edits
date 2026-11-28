# Downloads what OCR needs into vendor/ocr (not committed):
# - onnxruntime.dll 1.31.1 (MIT), taken from the official PyPI wheel. It is the
#   same build arabic-ocr runs, so parity tests compare like with like. The `ort`
#   crate loads it at run time (feature load-dynamic), on the first OCR.
# - The arabic-ocr models (CC BY-SA 4.0) at its v1.0.0 commit.
# - lexicon.tsv.gz (optional word list) from a local arabic-ocr checkout, if one
#   is found ($env:ARABIC_OCR_DIR, or ../arabic-ocr). It is not on GitHub.
$ErrorActionPreference = "Stop"
$OrtVersion = "1.31.1"
$OrtWheel = "https://files.pythonhosted.org/packages/59/bf/a1d282b4c753ba3e9b31f4c11abd2bc2390513dc5e3792ca3b78e42401c3/onnxruntime-1.31.1-cp313-cp313-win_amd64.whl"
$OrtSha256 = "0558f29d5f6ac09957390c9c5495931485ee8ae7f0ce781ccee86a0ae5c2348b"
$ModelCommit = "54951a3"
$Models = @{
  "detector-v2.onnx"         = "3040aeabced14750f59c8bcc9c38e166d6905ce1762db71969eab3d8d7484e8a"
  "recognizer-v6b-int8.onnx" = "3deb6c6f7b44e30e400a076c06184387fdd140aca562ab7fe64287f6b2668ec8"
}
$LexiconSha256 = "2f323ddde785c4aa292a824062b30b409c541e7b569493cca1223fd288070b40"

$root = Split-Path $PSScriptRoot -Parent
$dest = Join-Path $root "vendor\ocr"
New-Item -ItemType Directory -Force $dest | Out-Null

function Test-Hash($path, $sha) {
  (Test-Path $path) -and ((Get-FileHash $path -Algorithm SHA256).Hash.ToLower() -eq $sha)
}

$stamp = Join-Path $dest "ORT_VERSION.edits"
if ((Test-Path $stamp) -and ((Get-Content $stamp) -eq $OrtVersion) -and (Test-Path (Join-Path $dest "onnxruntime.dll"))) {
  Write-Host "ONNX Runtime $OrtVersion already in vendor/ocr"
} else {
  $whl = Join-Path $dest "ort.zip"
  Write-Host "Downloading ONNX Runtime $OrtVersion"
  Invoke-WebRequest -Uri $OrtWheel -OutFile $whl -UseBasicParsing
  if (-not (Test-Hash $whl $OrtSha256)) { throw "ONNX Runtime checksum mismatch" }
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $zip = [System.IO.Compression.ZipFile]::OpenRead($whl)
  try {
    $want = @{
      "onnxruntime/capi/onnxruntime.dll"   = "onnxruntime.dll"
      "onnxruntime/LICENSE"                = "onnxruntime-LICENSE.txt"
      "onnxruntime/ThirdPartyNotices.txt"  = "onnxruntime-ThirdPartyNotices.txt"
    }
    foreach ($e in $zip.Entries) {
      if ($want.ContainsKey($e.FullName)) {
        [System.IO.Compression.ZipFileExtensions]::ExtractToFile($e, (Join-Path $dest $want[$e.FullName]), $true)
      }
    }
  } finally { $zip.Dispose() }
  Remove-Item $whl
  if (-not (Test-Path (Join-Path $dest "onnxruntime.dll"))) { throw "onnxruntime.dll not found in the wheel" }
  Set-Content -Path $stamp -Value $OrtVersion
}

foreach ($name in $Models.Keys) {
  $path = Join-Path $dest $name
  if (Test-Hash $path $Models[$name]) { continue }
  Write-Host "Downloading $name"
  Invoke-WebRequest -Uri "https://raw.githubusercontent.com/youssef-mhmd/arabic-ocr/$ModelCommit/models/$name" -OutFile $path -UseBasicParsing
  if (-not (Test-Hash $path $Models[$name])) { throw "$name checksum mismatch" }
}

$lexicon = Join-Path $dest "lexicon.tsv.gz"
if (-not (Test-Hash $lexicon $LexiconSha256)) {
  $src = if ($env:ARABIC_OCR_DIR) { $env:ARABIC_OCR_DIR } else { Join-Path (Split-Path $root -Parent) "arabic-ocr" }
  $from = Join-Path $src "models\lexicon.tsv.gz"
  if (Test-Hash $from $LexiconSha256) {
    Copy-Item $from $lexicon
  } else {
    Write-Warning "lexicon.tsv.gz not found (set ARABIC_OCR_DIR); word correction will be off"
  }
}
Write-Host "OCR files ready in vendor/ocr"
