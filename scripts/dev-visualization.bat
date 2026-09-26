@echo off
REM Runs oqci-server and the visualization frontend together, and guarantees
REM both are stopped together.
REM
REM cmd.exe batch cannot reliably trap Ctrl+C to run cleanup code, so unlike
REM the .sh/.ps1 versions this script does not try to: each server gets its
REM own titled console window, and this script waits for a keypress before
REM tearing both down by window title with `taskkill /T` — which kills each
REM window's whole process tree, including the node/vite child `npm run dev`
REM spawns.
REM
REM See docs/visualization.md and server/README.md.
REM
REM Usage:
REM   scripts\dev-visualization.bat [--root DIR] [--backend ID]
REM       [--server-port PORT] [--frontend-port PORT]

setlocal enabledelayedexpansion

set "ROOT=."
set "BACKEND=simulator-nisq"
set "SERVER_PORT=4173"
set "FRONTEND_PORT=5173"

:parse_args
if "%~1"=="" goto args_done
if /I "%~1"=="--root" (
    set "ROOT=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="--backend" (
    set "BACKEND=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="--server-port" (
    set "SERVER_PORT=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="--frontend-port" (
    set "FRONTEND_PORT=%~2"
    shift
    shift
    goto parse_args
)
if /I "%~1"=="-h" goto usage
if /I "%~1"=="--help" goto usage
echo Unknown argument: %~1
goto usage
:args_done

set "REPO_ROOT=%~dp0.."
set "FRONTEND_DIR=%REPO_ROOT%\frontend"

if not exist "%FRONTEND_DIR%\package.json" (
    echo error: %FRONTEND_DIR%\package.json not found - run "npm install" in frontend\ first
    exit /b 1
)

echo Starting oqci-server on port %SERVER_PORT% ^(backend: %BACKEND%^)...
start "oqci-server" cmd /k "cd /d "%REPO_ROOT%" && cargo run -p oqci-server -- --root "%ROOT%" --backend "%BACKEND%" --port %SERVER_PORT%"

echo Starting frontend dev server on port %FRONTEND_PORT%...
start "oqci-frontend" cmd /k "cd /d "%FRONTEND_DIR%" && npm run dev -- --port %FRONTEND_PORT%"

echo.
echo   server:   http://localhost:%SERVER_PORT%
echo   frontend: http://localhost:%FRONTEND_PORT%
echo.
echo Both are running in their own windows, titled "oqci-server" and
echo "oqci-frontend". Come back to THIS window and press any key when you
echo want to stop both together.
echo.
pause >nul

echo Stopping both...
taskkill /FI "WINDOWTITLE eq oqci-server*" /T /F >nul 2>&1
taskkill /FI "WINDOWTITLE eq oqci-frontend*" /T /F >nul 2>&1
echo Done.

endlocal
exit /b 0

:usage
echo Usage: %~n0 [--root DIR] [--backend ID] [--server-port PORT] [--frontend-port PORT]
endlocal
exit /b 1
