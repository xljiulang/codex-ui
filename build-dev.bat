@echo off
setlocal
cd /d "%~dp0"

echo ============================================
echo   codex-ui DEV MODE (Vite HMR + Tauri)
echo ============================================

echo.
echo [0/2] Preflight: checking leftover dev processes ...

rem Close stale app windows first: an old codex-ui window keeps showing the previous UI
tasklist /FI "IMAGENAME eq codex-ui.exe" 2>nul | findstr /i "codex-ui.exe" >nul
if not errorlevel 1 (
  echo   [WARN] Detected a running codex-ui window which still shows the old UI.
  choice /c YN /m "Close these windows automatically and continue"
  if errorlevel 2 (
    echo   [ABORT] Please close all codex-ui windows manually, then rerun this script.
    exit /b 1
  )
  taskkill /F /IM codex-ui.exe >nul 2>nul
  echo   [OK] Closed old codex-ui processes.
)

rem Release port 5173: a leftover Vite dev server makes strictPort fail on the next run
set PORT_PID=
for /f "tokens=5" %%P in ('netstat -ano ^| findstr ":5173" ^| findstr "LISTENING"') do set PORT_PID=%%P
if defined PORT_PID (
  echo   [WARN] Port 5173 is occupied by PID %PORT_PID% - usually a leftover Vite dev server.
  taskkill /F /PID %PORT_PID% >nul 2>nul
  if errorlevel 1 (
    echo   [ERROR] Cannot kill PID %PORT_PID%. Close it manually and rerun this script.
    exit /b 1
  )
  echo   [OK] Port 5173 released.
)

where npm >nul 2>nul
if errorlevel 1 (
  echo [ERROR] npm not found. Install Node.js 18+ and add it to PATH.
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

rem WeChat sidecar deps (wechat-channel) live under sidecar\package.json;
rem build:sidecar needs them to bundle, so install once if missing.
if not exist "sidecar\node_modules\wechat-channel" (
  echo [1/3] Installing WeChat sidecar dependencies ^(npm install --prefix sidecar^) ...
  call npm install --prefix sidecar
  if errorlevel 1 (
    echo [ERROR] sidecar npm install failed.
    exit /b 1
  )
) else (
  echo [1/3] sidecar dependencies present, skipping install
)

echo [2/3] Building WeChat sidecar ^(npm run build:sidecar^) ...
call npm run build:sidecar
if errorlevel 1 (
  echo [ERROR] sidecar build failed.
  exit /b 1
)

echo [3/3] Starting npm run tauri dev ...
echo   UI loads from http://localhost:5173 with HMR; keep this terminal open.
echo   Running target\debug\codex-ui.exe directly loads the embedded dist/ and
echo   will NOT reflect recent UI changes.
call npm run build
call npm run tauri dev
if errorlevel 1 (
  echo [ERROR] tauri dev failed.
  exit /b 1
)

endlocal
exit /b 0
