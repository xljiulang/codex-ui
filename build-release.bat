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

rem ---- [1/5] Dependencies -------------------------------------------------
rem Legacy node_modules cache may lack new deps (qrcode runtime); probe and
rem install once if absent.
set NEED_INSTALL=0
if not exist node_modules set NEED_INSTALL=1
if exist node_modules if not exist "node_modules\qrcode" set NEED_INSTALL=1
if "%NEED_INSTALL%"=="1" (
  echo [1/5] Installing dependencies ^(npm install^) ...
  call npm install
  if errorlevel 1 (
    echo [ERROR] npm install failed.
    exit /b 1
  )
) else (
  echo [1/5] Dependencies present, skipping npm install
)

echo [2/5] Building frontend (npm run build -^> dist/) ...
call npm run build
if errorlevel 1 (
  echo [ERROR] Frontend build failed.
  exit /b 1
)

echo [3/5] Building Rust backend (cargo build --release) ...
call cargo build --release --manifest-path src-tauri\Cargo.toml
if errorlevel 1 (
  echo [ERROR] Rust build failed.
  exit /b 1
)

echo [4/5] Staging installer payload (copy exe into setup\) ...

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

rem 知识库向量推理所需的 ONNX Runtime（随包分发到 {app}\bin）：缺失时从 NuGet 取。
rem 取不到只告警不阻断——缺 DLL 只影响知识库检索，应用其余功能照常。
if not exist "setup\bin\onnxruntime.dll" (
  echo [4/5] Fetching onnxruntime.dll ^(ONNX Runtime 1.24+^) ...
  where pwsh >nul 2>nul
  if errorlevel 1 (
    powershell -NoProfile -ExecutionPolicy Bypass -File "scripts\fetch-onnxruntime.ps1"
  ) else (
    pwsh -NoProfile -File "scripts\fetch-onnxruntime.ps1"
  )
  if errorlevel 1 echo [WARN] onnxruntime.dll not ready; knowledge base retrieval will report missing runtime.
)

echo [5/5] Compiling installer (Inno Setup) ...
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
