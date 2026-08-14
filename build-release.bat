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

echo [3/3] Building Rust backend (cargo build --release) ...
call cargo build --release --manifest-path src-tauri\Cargo.toml
if errorlevel 1 (
  echo [ERROR] Rust build failed.
  exit /b 1
)

echo.
echo Build OK! Output: src-tauri\target\release\codex-ui.exe
echo For NSIS/MSI installer, run: npm run tauri build
echo.

endlocal
exit /b 0
