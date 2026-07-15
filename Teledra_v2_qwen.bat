@echo off
REM ── Teledra v2 (qwen2.5) — parallel model-swap launcher ──────────────────
REM Runs the same Teledra binary but points the brain at config.qwen.json
REM (qwen2.5:7b via Ollama) so you can feel how qwen2 performs on this machine.
REM
REM NOTE: this shares the knowledge/ state with the normal build, so run it as
REM an A/B model test (ideally when the live llama3 build is closed), not as a
REM second simultaneous instance. Requires `ollama pull qwen2.5:7b` to be done.
REM
REM %~dp0 is this script's own directory, so the launcher works from any
REM checkout rather than only from the one workstation it was written on.
set "TELEDRA_ROOT=%~dp0"
if "%TELEDRA_ROOT:~-1%"=="\" set "TELEDRA_ROOT=%TELEDRA_ROOT:~0,-1%"
cd /d "%TELEDRA_ROOT%"
set TELEDRA_CONFIG=config.qwen.json
title Teledra v2 (qwen2.5)
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%TELEDRA_ROOT%\tools\check_release_freshness.ps1" -ProjectRoot "%TELEDRA_ROOT%"
if errorlevel 1 (
    echo.
    pause
    exit /b 1
)
"%TELEDRA_ROOT%\target\release\teledra.exe"
