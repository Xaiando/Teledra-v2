@echo off
setlocal
cd /d "%~dp0"
set "FLY_PYTHON=python"
if exist "%~dp0.venv\Scripts\python.exe" set "FLY_PYTHON=%~dp0.venv\Scripts\python.exe"
if not exist "%~dp0.venv\Scripts\python.exe" if exist "%~dp0..\.venv\Scripts\python.exe" set "FLY_PYTHON=%~dp0..\.venv\Scripts\python.exe"
"%FLY_PYTHON%" run.py %*
if errorlevel 1 pause
