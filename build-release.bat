@echo off
setlocal
cd /d "%~dp0"

echo ============================================
echo   codex-ui RELEASE BUILD
echo ============================================

where npm >nul 2>nul
if errorlevel 1 (
  echo [ERROR] npm not found. Install Node.js 18+ and add it to PATH.
  exit /b 1
)

where cargo >nul 2>nul
if errorlevel 1 (
  echo [ERROR] cargo not found. Install Rust stable-msvc and add it to PATH.
  exit /b 1
)

if not exist node_modules (
  echo [1/3] Installing dependencies ^(npm install^) ...
  call npm install
  if errorlevel 1 (
    echo [ERROR] npm install failed.
    exit /b 1
  )
) else (
  echo [1/3] node_modules exists, skipping npm install
)

echo [2/3] Building frontend (npm run build -^> dist/) ...
call npm run build
if errorlevel 1 (
  echo [ERROR] Frontend build failed.
  exit /b 1
)

echo [3/4] Building Rust backend (cargo build --release) ...
call cargo build --release --manifest-path src-tauri\Cargo.toml
if errorlevel 1 (
  echo [ERROR] Rust build failed.
  exit /b 1
)

echo [4/4] Packaging installer (copy exe + Inno Setup) ...

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

copy /Y "src-tauri\target\release\codex-ui.exe" "setup\codex-ui.exe" >nul
if errorlevel 1 (
  echo [ERROR] Failed to copy codex-ui.exe to setup\.
  exit /b 1
)

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
