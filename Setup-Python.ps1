$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$Environment = Join-Path $Root ".venv"
$Python = Join-Path $Environment "Scripts\python.exe"
$SidecarRequirements = Join-Path $Root "Sidecars\requirements-optional.txt"
$LuxTtsRevision = "381b1609cb1d1afbf756b87809c623464cfd8ac5"

if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
    throw "uv is required. Install it with: winget install Astral-sh.uv"
}

uv venv $Environment --python 3.12 --clear
if ($LASTEXITCODE -ne 0) { throw "Could not create the Teledra Python environment" }

uv pip install --python $Python "git+https://github.com/ysharma3501/LuxTTS.git@$LuxTtsRevision"
if ($LASTEXITCODE -ne 0) { throw "Could not install the pinned LuxTTS runtime" }

uv pip install --python $Python --requirements $SidecarRequirements
if ($LASTEXITCODE -ne 0) { throw "Could not install optional sidecar packages" }

# Both the pinned LuxTTS package and the CUDA Torch wheel require the last
# setuptools 80 release. Keeping this explicit prevents later sidecar
# installs from silently upgrading the shared runtime past their contract.
uv pip install --python $Python "setuptools==80.9.0"
if ($LASTEXITCODE -ne 0) { throw "Could not pin the compatible setuptools runtime" }

uv pip check --python $Python
if ($LASTEXITCODE -ne 0) { throw "The Teledra Python environment has incompatible packages" }

$env:HF_HUB_DOWNLOAD_TIMEOUT = "120"
$env:HF_HUB_ETAG_TIMEOUT = "30"
& $Python -c "from huggingface_hub import snapshot_download; snapshot_download('YatharthS/LuxTTS'); snapshot_download('openai/whisper-base'); snapshot_download('Systran/faster-whisper-base.en'); print('Teledra voice models cached')"
if ($LASTEXITCODE -ne 0) { throw "Could not cache the LuxTTS and Whisper models" }

& $Python -c "import aiohttp, cv2, faster_whisper, mediapipe, PIL, pyaudio, pytesseract, sounddevice, speech_recognition, torch, youtube_transcript_api, zipvoice; print('Teledra Python environment ready')"
if ($LASTEXITCODE -ne 0) { throw "The Teledra Python environment failed its import check" }

$Tesseract = Get-Command tesseract.exe -ErrorAction SilentlyContinue
if (-not $Tesseract -and -not (Test-Path -LiteralPath "C:\Program Files\Tesseract-OCR\tesseract.exe")) {
    Write-Warning "Tesseract OCR is not installed. Install it with: winget install --id UB-Mannheim.TesseractOCR --exact"
}
