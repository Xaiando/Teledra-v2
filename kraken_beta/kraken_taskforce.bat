@echo off
rem ============================================================
rem  KRAKEN TASKFORCE — one-click launch
rem  Starts: mode hub (127.0.0.1:3838) + desktop chat room
rem          + 2 local workers (qwen / ornith, fully local)
rem  Drive it: type   <skill>: <input>   in Mission Chat, e.g.
rem      research_local: what does the swarm design require?
rem  A worker claims it, grinds it, signals the vault path back.
rem ============================================================
set HUB_EXE=C:\Users\Kaged\Documents\Agent orchestration\target\debug\agent-hub.exe
set DESKTOP_EXE=C:\Users\Kaged\Documents\Agent orchestration\target\debug\desktop.exe
set PY=D:\Teledra\.venv\Scripts\python.exe
set KRAKEN=D:\Teledra\kraken

rem -- optional arg 1: the work folder the taskforce gets free rein in ----
rem    usage: kraken_taskforce.bat D:\SomeProject
set WORKSPACE=%~1
if "%WORKSPACE%"=="" set WORKSPACE=%KRAKEN%\workspace

rem -- mode hub API (no-op if one already holds the port) ------
start "kraken-mode-hub" /min "%HUB_EXE%" --addr 127.0.0.1:3838 --data "%KRAKEN%\hub\data\hub_state.json"

rem -- workers (skip if already running: they double up harmlessly
rem    on missions thanks to shared-lock claim, but two is plenty) --
tasklist /fi "WINDOWTITLE eq kraken-worker-1*" 2>nul | find /i "python" >nul
if errorlevel 1 start "kraken-worker-1" /min cmd /c "cd /d %KRAKEN% && "%PY%" kraken.py worker kraken-worker-1 "%WORKSPACE%""
tasklist /fi "WINDOWTITLE eq kraken-worker-2*" 2>nul | find /i "python" >nul
if errorlevel 1 start "kraken-worker-2" /min cmd /c "cd /d %KRAKEN% && "%PY%" kraken.py worker kraken-worker-2 "%WORKSPACE%""

rem -- desktop chat room, pointed at the MODE state (cwd trick:
rem    desktop reads data\hub_state.json relative to its cwd) ----
cd /d "%KRAKEN%\hub"
start "kraken-taskforce-room" "%DESKTOP_EXE%"

echo.
echo  Kraken taskforce launching:
echo    room    : desktop window (this is the taskforce UI)
echo    api     : http://127.0.0.1:3838  (for agents/workers only, not for humans)
echo    workers : kraken-worker-1, kraken-worker-2 (minimized)
echo    workdir : %WORKSPACE%  (free-rein zone)
echo.
echo  In Mission Chat type:   ^<skill^>: ^<input^>
echo  Skills: research_local, research_web, research_fanout,
echo          code_forge, prod_digest, prod_vault
timeout /t 8 >nul
