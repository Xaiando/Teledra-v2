@echo off
setlocal
cd /d "%~dp0"
title Teledra V2 (qwen2.5)
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
rem Stop a running court before building. Cargo cannot replace a locked
rem executable, so a court left running turns every launch into a build
rem failure -- and the fallback below then starts the PREVIOUS binary. That is
rem how a session came to be spent looking at a TUI without the change that
rem had just been made to it. One court per machine is the rule anyway; the
rem application enforces the same thing once it starts.
taskkill /IM teledra.exe /F >nul 2>&1

set "TELEDRA_CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
if exist "%TELEDRA_CARGO%" (
    echo Building Teledra V2...
    "%TELEDRA_CARGO%" build --release -p teledra
    if errorlevel 1 (
        echo Teledra V2 could not be built; the existing build was left alone.
        if not exist "%TELEDRA_EXE%" (
            pause
            exit /b 1
        )
        echo.
        echo ***********************************************************
        echo *  WARNING: launching the PREVIOUS build. The change you  *
        echo *  just made is NOT in it. Check the Build line in the    *
        echo *  backstage trace to see which one is running.           *
        echo ***********************************************************
        echo.
        pause
    )
) else if not exist "%TELEDRA_EXE%" (
    echo Teledra V2 could not find the installed application or Cargo.
    pause
    exit /b 1
)

"%TELEDRA_EXE%" %*
set "TELEDRA_EXIT=%ERRORLEVEL%"
if not "%TELEDRA_EXIT%"=="0" pause
exit /b %TELEDRA_EXIT%
