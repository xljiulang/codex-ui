@echo off
setlocal
cd /d "%~dp0"

rem ============================================================
rem  codex-ui INSTALLER PACKAGING (Inno Setup only)
rem
rem  Only the last step: compile the installer from an existing
rem  setup\codex-ui.exe. It does NOT run npm build / cargo build
rem  (the Rust release build is done by GitHub Actions now).
rem
rem  Usage:
rem    build-installer.bat                  use existing setup\codex-ui.exe
rem    build-installer.bat <path to exe>    copy it into setup\ first
rem
rem  For the full local build chain, keep using build-release.bat.
rem  NOTE: this file must stay ASCII-only - cmd.exe reads batch files
rem  with the OEM code page, so non-ASCII comments break parsing.
rem ============================================================

echo ============================================
echo   codex-ui INSTALLER (Inno Setup only)
echo ============================================

if not "%~1"=="" (
  if not exist "%~1" (
    echo [ERROR] File not found: %~1
    exit /b 1
  )
  echo [1/2] Staging codex-ui.exe from %~1 ...
  copy /Y "%~1" "setup\codex-ui.exe" >nul
  if errorlevel 1 (
    echo [ERROR] Failed to copy into setup\codex-ui.exe.
    exit /b 1
  )
) else (
  echo [1/2] Using existing setup\codex-ui.exe
)

if not exist "setup\codex-ui.exe" (
  echo [ERROR] setup\codex-ui.exe not found.
  echo         Download the codex-ui.exe artifact from GitHub Actions ^(build-release workflow^),
  echo         then either pass its path to this script:
  echo             build-installer.bat "%%USERPROFILE%%\Downloads\codex-ui.exe"
  echo         or copy it to setup\codex-ui.exe manually.
  echo         To build everything on this machine instead, run build-release.bat.
  exit /b 1
)

rem Locate ISCC.exe: PATH first, then common install directories
set ISCC=
for /f "delims=" %%I in ('where ISCC.exe 2^>nul') do if not defined ISCC set "ISCC=%%I"
if not defined ISCC if exist "%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe" set "ISCC=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if not defined ISCC if exist "%ProgramFiles%\Inno Setup 6\ISCC.exe" set "ISCC=%ProgramFiles%\Inno Setup 6\ISCC.exe"
if not defined ISCC if exist "%LocalAppData%\Programs\Inno Setup 6\ISCC.exe" set "ISCC=%LocalAppData%\Programs\Inno Setup 6\ISCC.exe"
if not defined ISCC (
  echo [ERROR] Inno Setup 6 not found. Install it and add ISCC.exe to PATH.
  exit /b 1
)

echo [2/2] Compiling installer (Inno Setup) ...
"%ISCC%" "setup\setup.iss"
if errorlevel 1 (
  echo [ERROR] Inno Setup compilation failed.
  exit /b 1
)

echo.
echo Build OK! Installer: setup\output\codex-ui-win-x64.exe
echo.

endlocal
exit /b 0
