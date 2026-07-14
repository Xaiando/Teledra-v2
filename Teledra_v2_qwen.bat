@echo off
REM ── Teledra v2 (qwen2.5) — parallel model-swap launcher ──────────────────
REM Runs the same Teledra binary but points the brain at config.qwen.json
REM (qwen2.5:7b via Ollama) so you can feel how qwen2 performs on this machine.
REM
REM NOTE: this shares the knowledge/ state with the normal build, so run it as
REM an A/B model test (ideally when the live llama3 build is closed), not as a
REM second simultaneous instance. Requires `ollama pull qwen2.5:7b` to be done.
cd /d D:\Teledra
set TELEDRA_CONFIG=config.qwen.json
title Teledra v2 (qwen2.5)
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "D:\Teledra\tools\check_release_freshness.ps1" -ProjectRoot "D:\Teledra"
if errorlevel 1 (
    echo.
    pause
    exit /b 1
)
"D:\Teledra\target\release\teledra.exe"
