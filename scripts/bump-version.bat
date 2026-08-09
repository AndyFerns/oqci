@echo off
setlocal enabledelayedexpansion
rem OQCI version bumper (Windows / cmd.exe).
rem
rem The repo-root VERSION file is the single source of truth for the project
rem version (semantic versioning, MAJOR.MINOR.PATCH). Run this whenever you cut
rem a change worth versioning; it bumps VERSION and keeps Cargo.toml's package
rem version in sync so the two can never drift.
rem
rem When to bump what (semantic versioning):
rem   major  - breaking change to a public API or IR contract (0.x: still allowed)
rem   minor  - new backwards-compatible capability / milestone (e.g. a new phase)
rem   patch  - fixes, docs, internal changes with no API surface change
rem
rem Usage:
rem   scripts\bump-version.bat patch          0.0.1 -^> 0.0.2
rem   scripts\bump-version.bat minor          0.0.1 -^> 0.1.0
rem   scripts\bump-version.bat major          0.9.3 -^> 1.0.0
rem   scripts\bump-version.bat set 1.2.3      set an explicit version
rem   scripts\bump-version.bat --show         print the current version
rem   scripts\bump-version.bat -h ^| --help
rem
rem On success it prints `old -> new` and a suggested tag command (it does NOT
rem create tags or commits -- that stays your decision).
rem
rem Exit codes: 0 ok  2 bad arguments  3 malformed VERSION / file error

set "SCRIPT_DIR=%~dp0"
pushd "%SCRIPT_DIR%.." >nul
set "ROOT=%CD%"
set "VERSION_FILE=%ROOT%\VERSION"
set "CARGO_FILE=%ROOT%\Cargo.toml"

if "%~1"=="" (
    echo error: missing argument 1^>^&2
    call :usage
    popd >nul
    exit /b 2
)
if /i "%~1"=="-h"     (call :usage & popd >nul & exit /b 0)
if /i "%~1"=="--help" (call :usage & popd >nul & exit /b 0)

rem --- read + validate current version ---------------------------------------

if not exist "%VERSION_FILE%" (
    echo error: VERSION file not found at %VERSION_FILE% 1^>^&2
    popd >nul & exit /b 3
)
set "CURRENT="
for /f "usebackq tokens=* delims= " %%V in ("%VERSION_FILE%") do if not defined CURRENT set "CURRENT=%%V"
rem strip stray whitespace
set "CURRENT=%CURRENT: =%"
if "%CURRENT%"=="" (
    echo error: VERSION file is empty 1^>^&2
    popd >nul & exit /b 3
)

rem validate MAJOR.MINOR.PATCH (all numeric)
for /f "tokens=1-4 delims=." %%a in ("%CURRENT%") do (
    set "MAJOR=%%a"
    set "MINOR=%%b"
    set "PATCH=%%c"
    set "EXTRA=%%d"
)
if defined EXTRA goto :bad_semver
call :is_number "%MAJOR%" || goto :bad_semver
call :is_number "%MINOR%" || goto :bad_semver
call :is_number "%PATCH%" || goto :bad_semver

rem --- compute new version ----------------------------------------------------

if /i "%~1"=="--show" (
    echo %CURRENT%
    popd >nul & exit /b 0
)
if /i "%~1"=="major" (set /a NM=MAJOR+1 & set "NEW=!NM!.0.0" & goto :apply)
if /i "%~1"=="minor" (set /a NM=MINOR+1 & set "NEW=%MAJOR%.!NM!.0" & goto :apply)
if /i "%~1"=="patch" (set /a NM=PATCH+1 & set "NEW=%MAJOR%.%MINOR%.!NM!" & goto :apply)
if /i "%~1"=="set" (
    if "%~2"=="" (echo error: 'set' needs a version, e.g. 'set 1.2.3' 1^>^&2 & popd ^>nul & exit /b 2)
    set "NEW=%~2"
    for /f "tokens=1-4 delims=." %%a in ("!NEW!") do (set "SA=%%a" & set "SB=%%b" & set "SC=%%c" & set "SD=%%d")
    if defined SD (echo error: '!NEW!' is not valid semver 1^>^&2 & popd ^>nul & exit /b 2)
    call :is_number "!SA!" || (echo error: '!NEW!' is not valid semver 1^>^&2 & popd ^>nul & exit /b 2)
    call :is_number "!SB!" || (echo error: '!NEW!' is not valid semver 1^>^&2 & popd ^>nul & exit /b 2)
    call :is_number "!SC!" || (echo error: '!NEW!' is not valid semver 1^>^&2 & popd ^>nul & exit /b 2)
    goto :apply
)

echo error: unknown bump kind: %~1 1^>^&2
call :usage
popd >nul & exit /b 2

:apply
rem --- write VERSION ----------------------------------------------------------
> "%VERSION_FILE%" echo !NEW!
if errorlevel 1 (echo error: could not write %VERSION_FILE% 1^>^&2 & popd ^>nul & exit /b 3)

rem --- sync Cargo.toml [package] version --------------------------------------
rem Only the package version line changes; dependency version strings like
rem `thiserror = "2"` are left alone because they do not start with `version `.
if exist "%CARGO_FILE%" (
    set "TMP_CARGO=%TEMP%\oqci-cargo-%RANDOM%.toml"
    set "REPLACED=0"
    > "!TMP_CARGO!" (
        for /f "usebackq delims=" %%L in (`findstr /n "^" "%CARGO_FILE%"`) do (
            set "raw=%%L"
            rem strip the leading "<n>:" that findstr /n adds, keeping colons in content
            set "content="
            for /f "tokens=1* delims=:" %%m in ("!raw!") do set "content=%%n"
            set "trimmed=!content: =!"
            rem decide whether this is the (first) package version line
            set "isver=0"
            if "!REPLACED!"=="0" if "!trimmed:~0,8!"=="version=" set "isver=1"
            if "!isver!"=="1" (
                echo version = "!NEW!"
                set "REPLACED=1"
            ) else (
                echo(!content!
            )
        )
    )
    move /y "!TMP_CARGO!" "%CARGO_FILE%" >nul
    if errorlevel 1 (echo error: could not update %CARGO_FILE% 1^>^&2 & popd ^>nul & exit /b 3)
)

echo version: %CURRENT% -^> !NEW!
echo updated: VERSION, Cargo.toml
echo next (optional): git commit -am "chore: bump version to !NEW!" ^&^& git tag v!NEW!
popd >nul
exit /b 0

rem ------------------------------------------------------------------ helpers

:bad_semver
echo error: VERSION '%CURRENT%' is not valid semver (MAJOR.MINOR.PATCH) 1^>^&2
popd >nul
exit /b 3

rem is_number <str> -> exit 0 if all-digits and non-empty, else exit 1
:is_number
set "N=%~1"
if "%N%"=="" exit /b 1
for /f "delims=0123456789" %%x in ("%N%") do exit /b 1
exit /b 0

:usage
for /f "usebackq tokens=* delims=" %%L in ("%~f0") do (
    set "line=%%L"
    if defined line (
        set "first=!line:~0,3!"
        if "!first!"=="rem" echo(!line:~4!
    )
)
exit /b 0
