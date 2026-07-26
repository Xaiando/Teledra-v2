@echo off
setlocal
cd /d "%~dp0"

set "TELEDRA_CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
if not exist "%TELEDRA_CARGO%" (
    echo Teledra V2 could not find Cargo at:
    echo   %TELEDRA_CARGO%
    exit /b 1
)

"%TELEDRA_CARGO%" run --release -p teledra -- %*
exit /b %ERRORLEVEL%

