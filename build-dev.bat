@echo off
setlocal
cd /d "%~dp0"

echo ============================================
echo   codex-ui DEV MODE (Vite HMR + Tauri)
echo ============================================

where npm >nul 2>nul
if errorlevel 1 (
  echo [ERROR] npm not found. Install Node.js 18+ and add it to PATH.
  exit /b 1
)

if not exist node_modules (
  echo [1/2] Installing dependencies ^(npm install^) ...
  call npm install
  if errorlevel 1 (
    echo [ERROR] npm install failed.
    exit /b 1
  )
) else (
  echo [1/2] node_modules exists, skipping npm install
)

echo [2/2] Starting npm run tauri dev ...
call npm run tauri dev
if errorlevel 1 (
  echo [ERROR] tauri dev failed.
  exit /b 1
)

endlocal
exit /b 0
