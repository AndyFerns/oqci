@echo off
REM Force-stops oqci-server and the visualization frontend dev server, by
REM port and by process/window name - a safety net for when they weren't
REM stopped through dev-visualization.bat (a window was closed directly, a
REM crash, a manually-started instance, etc.).
REM
REM Usage:
REM   scripts\stop-visualization.bat [SERVER_PORT] [FRONTEND_PORT]

setlocal enabledelayedexpansion

set "SERVER_PORT=4173"
set "FRONTEND_PORT=5173"
if not "%~1"=="" set "SERVER_PORT=%~1"
if not "%~2"=="" set "FRONTEND_PORT=%~2"

call :kill_port %SERVER_PORT%
call :kill_port %FRONTEND_PORT%

echo Backstop: killing any oqci-server.exe by name...
taskkill /IM oqci-server.exe /T /F >nul 2>&1

echo Backstop: killing dev-visualization.bat's titled windows, if any remain...
taskkill /FI "WINDOWTITLE eq oqci-server*" /T /F >nul 2>&1
taskkill /FI "WINDOWTITLE eq oqci-frontend*" /T /F >nul 2>&1

echo Done.
endlocal
exit /b 0

:kill_port
set "PORT=%~1"
set "FOUND="
for /f "tokens=5" %%P in ('netstat -ano ^| findstr /R /C:":%PORT% .*LISTENING"') do (
    echo Killing PID %%P listening on port %PORT%
    taskkill /PID %%P /T /F >nul 2>&1
    set "FOUND=1"
)
if not defined FOUND echo Nothing listening on port %PORT%
exit /b 0
