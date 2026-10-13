@echo off
setlocal
cd /d "%~dp0"
rem Every office uses this Ollama model. qwen2.5-coder:14b fits a 16 GB GPU with
rem the 16K context (about 40 tokens/s); qwen2.5-coder:32b is too slow there
rem (about 5 tokens/s). qwen2.5:7b is the fast fallback.
if not defined TELEDRA_MODEL set "TELEDRA_MODEL=qwen2.5-coder:14b"
rem Qwen layers kept on the GPU; the rest run from system RAM. Qwen, the voice
rem engine (3.2 GB) and OBS share one 16 GB card, and overflowing it makes
rem Windows page GPU memory and drops Qwen from ~41 to ~2 tokens/s. 44 of 49
rem layers measured 27 tokens/s with 1.2 GB spare while streaming. Use 49 when
rem the NVIDIA Overlay is off and VRAM allows.
if not defined TELEDRA_GPU_LAYERS set "TELEDRA_GPU_LAYERS=44"
title Teledra V2 (%TELEDRA_MODEL%)
mode con: cols=140 lines=42 >nul 2>&1

if not defined TELEDRA_PYTHON if exist "%~dp0.venv\Scripts\python.exe" (
    set "TELEDRA_PYTHON=%~dp0.venv\Scripts\python.exe"
)

rem Build whenever Cargo is available, not only on the first launch. Cargo is a
rem no-op in well under a second when nothing changed, and the previous rule --
rem build only if the executable is missing -- meant every source change after
rem the first launch was silently ignored. A whole stream ran on a stale binary
rem that way, with fixes that were present in the tree and absent from the exe.
set "TELEDRA_EXE=%~dp0target\release\teledra.exe"
rem Inspect all arguments without altering %%*, which is forwarded unchanged.
rem Read-only views/checks use the installed build and leave a running court alone.
set "TELEDRA_READ_ONLY="
:inspect_arguments
if "%~1"=="" goto arguments_checked
if /I "%~1"=="--check" set "TELEDRA_READ_ONLY=1"
if /I "%~1"=="--check-environment" set "TELEDRA_READ_ONLY=1"
if /I "%~1"=="--portrait" set "TELEDRA_READ_ONLY=1"
if /I "%~1"=="--vertical-view" set "TELEDRA_READ_ONLY=1"
if /I "%~1"=="--dashboard-view" set "TELEDRA_READ_ONLY=1"
if /I "%~1"=="--work-view" set "TELEDRA_READ_ONLY=1"
shift
goto inspect_arguments
:arguments_checked
if defined TELEDRA_READ_ONLY if exist "%TELEDRA_EXE%" goto launch

rem Stop a running court before building. Cargo cannot replace a locked
rem executable, so a court left running turns every launch into a build
rem failure -- and the fallback below then starts the PREVIOUS binary. That is
rem how a session came to be spent looking at a TUI without the change that
rem had just been made to it. One court per machine is the rule anyway; the
rem application enforces the same thing once it starts.
powershell -NoProfile -Command "$teledraTarget = [IO.Path]::GetFullPath($env:TELEDRA_EXE); Get-Process -Name teledra -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $teledraTarget } | Stop-Process -Force -ErrorAction SilentlyContinue" >nul 2>&1

set "TELEDRA_CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
if exist "%TELEDRA_CARGO%" (
    echo Building Teledra V2...
    "%TELEDRA_CARGO%" build --release -p teledra
    if errorlevel 1 (
        echo Teledra V2 could not be built; the existing build was left alone.
        echo Fix the build error before launching so the court uses the current code.
        pause
        exit /b 1
    )
) else if not exist "%TELEDRA_EXE%" (
    echo Teledra V2 could not find the installed application or Cargo.
    pause
    exit /b 1
)

:launch
rem The interactive portrait court needs its foreground/background RGB colours.
rem Agent shells often export NO_COLOR=1; do not inherit that logging preference
rem into the visual application. setlocal keeps the parent environment intact.
rem Read-only diagnostics retain the caller's original colour preference.
if not defined TELEDRA_READ_ONLY set "NO_COLOR="
"%TELEDRA_EXE%" %*
set "TELEDRA_EXIT=%ERRORLEVEL%"
if not "%TELEDRA_EXIT%"=="0" pause
exit /b %TELEDRA_EXIT%
