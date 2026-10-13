@echo off
setlocal
cd /d "%~dp0"
py -3.12 -m venv .venv
if errorlevel 1 goto failed
".venv\Scripts\python.exe" -m pip install -r requirements.txt
if errorlevel 1 goto failed
".venv\Scripts\python.exe" run.py --check
if errorlevel 1 goto failed
echo Ready. Open FlyComposer.bat, then http://127.0.0.1:8789/ in your browser.
pause
exit /b 0
:failed
echo Setup did not complete. Python 3.12 for Windows is required.
pause
exit /b 1
