@echo off
setlocal enabledelayedexpansion
rem OQCI full-pipeline build script (Windows / cmd.exe).
rem
rem Runs every check the project cares about, in order:
rem   fmt-check ^| clippy ^| test ^| doctest ^| rustdoc ^| mdbook
rem
rem All stages route through cargo/mdbook, which auto-discover modules and
rem chapters -- this script does not enumerate them, so adding a new module,
rem integration test, or doc chapter never requires editing here.
rem
rem On failure it prints a single LLM-friendly summary block delimited by
rem "=====", one section per failed stage, with the exit code and the full
rem captured output. Copy the whole block into a chat to get help.
rem
rem Usage:
rem   scripts\build.bat                run everything
rem   scripts\build.bat --fix          auto-apply `cargo fmt` before checks
rem   scripts\build.bat --fast         skip mdbook (Rust checks only)
rem   scripts\build.bat --docs-only    just build the doc site
rem   scripts\build.bat --serve        after docs build, serve them locally
rem   scripts\build.bat -h ^| --help    show this help
rem
rem Exit codes:
rem   0  every non-skipped stage passed
rem   1  one or more stages failed (see summary block)
rem   2  bad arguments or missing required tool

rem ------------------------------------------------------------------ config

set "SCRIPT_DIR=%~dp0"
pushd "%SCRIPT_DIR%.." >nul
set "ROOT=%CD%"

set "FIX=0"
set "FAST=0"
set "DOCS_ONLY=0"
set "SERVE=0"

:parse_args
if "%~1"=="" goto args_done
if /i "%~1"=="--fix"        (set "FIX=1"       & shift & goto parse_args)
if /i "%~1"=="--fast"       (set "FAST=1"      & shift & goto parse_args)
if /i "%~1"=="--docs-only"  (set "DOCS_ONLY=1" & shift & goto parse_args)
if /i "%~1"=="--serve"      (set "SERVE=1"     & shift & goto parse_args)
if /i "%~1"=="-h"           goto show_help
if /i "%~1"=="--help"       goto show_help
echo error: unknown argument: %~1 1^>^&2
echo run '%~nx0 --help' for usage 1^>^&2
popd >nul
exit /b 2

:show_help
rem Print the header comment as help text.
for /f "usebackq tokens=* delims=" %%L in ("%~f0") do (
    set "line=%%L"
    if defined line (
        set "first=!line:~0,3!"
        if "!first!"=="rem" (
            set "trimmed=!line:~4!"
            echo(!trimmed!
        )
    )
)
popd >nul
exit /b 0

:args_done

rem ------------------------------------------------------------------ state

set "STAGE_TMP=%TEMP%\oqci-build-%RANDOM%%RANDOM%"
mkdir "%STAGE_TMP%" >nul 2>&1
set "STAGE_COUNT=0"
set "FAIL_COUNT=0"
set "SKIP_COUNT=0"

rem ------------------------------------------------------------------ prereq

where cargo >nul 2>&1
if errorlevel 1 (
    echo error: required tool 'cargo' not found on PATH 1^>^&2
    echo   install hint: install Rust via https://rustup.rs 1^>^&2
    rmdir /s /q "%STAGE_TMP%" >nul 2>&1
    popd >nul
    exit /b 2
)

echo == OQCI build pipeline ==
echo repo:      %ROOT%
for /f "delims=" %%V in ('rustc --version 2^>nul') do set "RUSTC_VER=%%V"
if not defined RUSTC_VER set "RUSTC_VER=unknown"
echo toolchain: %RUSTC_VER%
echo.

rem ------------------------------------------------------------------ stages

if "%DOCS_ONLY%"=="1" goto docs_stage

if "%FIX%"=="1" call :run_stage "fmt-apply" 0 cargo cargo fmt --all

call :run_stage "fmt-check" 0 cargo cargo fmt --all -- --check
call :run_stage "clippy"    0 cargo cargo clippy --all-targets --all-features -- -D warnings
call :run_stage "test"      0 cargo cargo test --all-targets --all-features
call :run_stage "doctest"   0 cargo cargo test --doc --all-features

rem rustdoc: set RUSTDOCFLAGS in the parent shell so the child inherits it.
set "SAVED_RUSTDOCFLAGS=%RUSTDOCFLAGS%"
set "RUSTDOCFLAGS=-D warnings"
call :run_stage "rustdoc"   0 cargo cargo doc --no-deps --all-features
set "RUSTDOCFLAGS=%SAVED_RUSTDOCFLAGS%"

:docs_stage
if "%FAST%"=="0" call :run_stage "mdbook" 1 mdbook mdbook build docs
if "%SERVE%"=="1" (
    where mdbook >nul 2>&1
    if not errorlevel 1 (
        echo.
        echo Serving docs at http://localhost:3000
        mdbook serve docs --open
        goto :cleanup_and_exit_0
    )
)

goto report

rem ------------------------------------------------------------------ report

:report
echo.
echo ============================================================
echo  BUILD SUMMARY
echo ============================================================

for /l %%I in (1,1,%STAGE_COUNT%) do call :report_one %%I

if "%FAIL_COUNT%"=="0" (
    echo.
    if "%SKIP_COUNT%"=="0" (
        echo all stages passed
    ) else (
        echo all stages passed ^(%SKIP_COUNT% skipped^)
    )
    goto cleanup_and_exit_0
)

echo.
echo ============================================================
echo BUILD FAILED -- %FAIL_COUNT% stage(s) failed
echo toolchain: %RUSTC_VER%
for /f "tokens=*" %%O in ('ver') do echo platform:  %%O
echo cwd:       %ROOT%
echo ============================================================

for /l %%I in (1,1,%STAGE_COUNT%) do call :dump_failure %%I

echo.
echo ============================================================
echo END BUILD FAILURE -- copy from 'BUILD FAILED' through this line
echo ============================================================

rmdir /s /q "%STAGE_TMP%" >nul 2>&1
popd >nul
exit /b 1

:cleanup_and_exit_0
rmdir /s /q "%STAGE_TMP%" >nul 2>&1
popd >nul
exit /b 0

rem ------------------------------------------------------------------ helpers

rem report_one <idx>  -- print one summary line
:report_one
set "IDX=%~1"
set "N=!NAME_%IDX%!"
set "S=!STATUS_%IDX%!"
set "T=!SECS_%IDX%!"
if "!S!"=="pass" goto report_pass
set "PREFIX=!S:~0,4!"
if "!PREFIX!"=="fail" goto report_fail
if "!PREFIX!"=="skip" goto report_skip
goto :eof

:report_pass
call :pad "!N!" 14
echo   [ok]      !PADDED!  !T!s
goto :eof

:report_fail
set "RC=!S:~5!"
call :pad "!N!" 14
echo   [FAILED]  !PADDED!  !T!s  (exit !RC!)
goto :eof

:report_skip
set "REASON=!S:~5!"
call :pad "!N!" 14
echo   [skipped] !PADDED!  (!REASON!)
goto :eof

rem dump_failure <idx>  -- print the failure block for one stage, if failed
:dump_failure
set "IDX=%~1"
set "S=!STATUS_%IDX%!"
set "PREFIX=!S:~0,4!"
if not "!PREFIX!"=="fail" goto :eof
set "N=!NAME_%IDX%!"
set "RC=!S:~5!"
set "LOG=!LOG_%IDX%!"
echo.
echo --- [!N!] exit=!RC! ---
if defined LOG (
    if exist "!LOG!" (
        type "!LOG!"
    ) else (
        echo ^(no captured output -- prerequisite check failed^)
    )
) else (
    echo ^(no captured output -- prerequisite check failed^)
)
goto :eof

rem run_stage <name> <optional?:0|1> <required-tool> <cmd> [args...]
:run_stage
set "S_NAME=%~1"
set "S_OPT=%~2"
set "S_TOOL=%~3"
shift & shift & shift

set /a STAGE_COUNT+=1
set "IDX=%STAGE_COUNT%"
set "NAME_%IDX%=%S_NAME%"

where %S_TOOL% >nul 2>&1
if errorlevel 1 (
    if "%S_OPT%"=="1" (
        echo [SKIP]    [%S_NAME%] tool not found: %S_TOOL%
        set "STATUS_%IDX%=skip:tool-missing:%S_TOOL%"
        set "SECS_%IDX%=0"
        set "LOG_%IDX%="
        set /a SKIP_COUNT+=1
        exit /b 0
    ) else (
        echo [FAILED]  [%S_NAME%] required tool not found: %S_TOOL%
        set "STATUS_%IDX%=fail:127"
        set "SECS_%IDX%=0"
        set "LOG_%IDX%="
        set /a FAIL_COUNT+=1
        exit /b 0
    )
)

rem Build a display string from remaining args.
set "S_DISPLAY="
:collect
if "%~1"=="" goto collected
if defined S_DISPLAY (set "S_DISPLAY=!S_DISPLAY! %~1") else (set "S_DISPLAY=%~1")
shift
goto collect
:collected

echo [RUN]     [%S_NAME%] !S_DISPLAY!

set "LOG=%STAGE_TMP%\%S_NAME%.log"
set "START=%TIME%"

rem Execute via cmd /c, redirecting to log. We then `type` the log so the user
rem still sees the output live-ish (post-execution, but present in the transcript).
cmd /c !S_DISPLAY! > "%LOG%" 2>&1
set "RC=!ERRORLEVEL!"

if exist "%LOG%" type "%LOG%"

call :elapsed "%START%" "%TIME%"

if "!RC!"=="0" (
    echo [ok]      [%S_NAME%] ^(!ELAPSED!s^)
    set "STATUS_%IDX%=pass"
    set "SECS_%IDX%=!ELAPSED!"
    set "LOG_%IDX%=%LOG%"
) else (
    echo [FAILED]  [%S_NAME%] exit !RC! ^(!ELAPSED!s^)
    set "STATUS_%IDX%=fail:!RC!"
    set "SECS_%IDX%=!ELAPSED!"
    set "LOG_%IDX%=%LOG%"
    set /a FAIL_COUNT+=1
)
exit /b 0

rem elapsed <start-time> <end-time>  -> sets ELAPSED (seconds, integer)
:elapsed
setlocal
set "T1=%~1"
set "T2=%~2"
for /f "tokens=1-4 delims=:.," %%a in ("%T1%") do set /a S1=(((%%a*60)+1%%b %% 100)*60+1%%c %% 100)
for /f "tokens=1-4 delims=:.," %%a in ("%T2%") do set /a S2=(((%%a*60)+1%%b %% 100)*60+1%%c %% 100)
set /a D=S2-S1
if %D% lss 0 set /a D+=86400
endlocal & set "ELAPSED=%D%"
exit /b 0

rem pad <string> <width>  -> sets PADDED (left-aligned, space-padded to width)
:pad
setlocal
set "STR=%~1"
set /a W=%~2
set "OUT=%STR%                                        "
set "OUT=!OUT:~0,%W%!"
endlocal & set "PADDED=%OUT%"
exit /b 0
