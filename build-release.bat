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

rem ---- [1/6] Dependencies -------------------------------------------------
rem Legacy node_modules cache may lack new deps (qrcode runtime,
rem @esbuild bundler); probe both, install once if any is absent.
set NEED_INSTALL=0
if not exist node_modules set NEED_INSTALL=1
if exist node_modules if not exist "node_modules\qrcode" set NEED_INSTALL=1
if exist node_modules if not exist "node_modules\@esbuild" set NEED_INSTALL=1
if "%NEED_INSTALL%"=="1" (
  echo [1/6] Installing dependencies ^(npm install^) ...
  call npm install
  if errorlevel 1 (
    echo [ERROR] npm install failed.
    exit /b 1
  )
) else (
  echo [1/6] Dependencies present, skipping npm install
)

rem WeChat sidecar deps (wechat-channel) live under sidecar\package.json;
rem build:sidecar needs them to bundle, so install once if missing.
if not exist "sidecar\node_modules\wechat-channel" (
  echo [1/6] Installing WeChat sidecar dependencies ^(npm install --prefix sidecar^) ...
  call npm install --prefix sidecar
  if errorlevel 1 (
    echo [ERROR] sidecar npm install failed.
    exit /b 1
  )
) else (
  echo [1/6] sidecar dependencies present, skipping install
)

echo [2/6] Building frontend (npm run build -^> dist/) ...
call npm run build
if errorlevel 1 (
  echo [ERROR] Frontend build failed.
  exit /b 1
)

echo [3/6] Building WeChat sidecar (npm run build:sidecar -^> sidecar-dist/) ...
call npm run build:sidecar
if errorlevel 1 (
  echo [ERROR] Sidecar build failed.
  exit /b 1
)

echo [4/6] Building Rust backend (cargo build --release) ...
call cargo build --release --manifest-path src-tauri\Cargo.toml
if errorlevel 1 (
  echo [ERROR] Rust build failed.
  exit /b 1
)

echo [5/6] Staging installer payload (copy exe + sidecar into setup\) ...

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

rem WeChat sidecar single-file bundle; layout matches Rust resolver
if not exist "setup\resources\wechat-sidecar" mkdir "setup\resources\wechat-sidecar"
copy /Y "sidecar-dist\wechat-sidecar.mjs" "setup\resources\wechat-sidecar\" >nul
if errorlevel 1 (
  echo [ERROR] Failed to copy WeChat sidecar into setup\resources.
  exit /b 1
)

echo [6/6] Compiling installer (Inno Setup) ...
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
