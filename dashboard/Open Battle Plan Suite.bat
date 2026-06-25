@echo off
title Battle Plan - Suite (Dashboard + Replay)
cd /d "%~dp0"
echo ================================================
echo    Battle Plan - SUITE (offline)
echo    Stats Dashboard + Game Replay on one server
echo ================================================
echo Starting the local server (127.0.0.1 only)...
echo A browser tab will open at http://127.0.0.1:8080
echo.
echo   Landing : http://127.0.0.1:8080/
echo   Stats   : http://127.0.0.1:8080/stats
echo   Replay  : http://127.0.0.1:8080/replay
echo.
echo Keep THIS window open while you use the UI.
echo Close it (or press Ctrl+C) to stop.
echo.
rem Find a REAL Python by full path first - this avoids the Microsoft Store
rem "App execution alias" stub that hijacks bare 'python'/'py' on many PCs.
set "PYEXE="
for %%P in (
  "C:\Program Files\Python311\python.exe"
  "C:\Program Files\Python312\python.exe"
  "C:\Program Files\Python313\python.exe"
  "C:\Program Files\Python310\python.exe"
  "%LOCALAPPDATA%\Programs\Python\Python311\python.exe"
  "%LOCALAPPDATA%\Programs\Python\Python312\python.exe"
  "C:\Windows\py.exe"
) do if not defined PYEXE if exist "%%~P" set "PYEXE=%%~P"
if not defined PYEXE (
  echo  *** Python 3 was not found in the usual places. ***
  echo  Install it from https://www.python.org/downloads/
  echo  and tick "Add python.exe to PATH" during setup, then try again.
  echo.
  pause
  exit /b
)
"%PYEXE%" launcher.py
echo.
echo (server stopped)
pause
