#define MyAppName "codex-ui"
#define MyAppExeName "codex-ui.exe"
#define MyAppVersion GetVersionNumbersString(".\codex-ui.exe")
#define MyAppGuid "{CF89B894-F2A3-4CAE-A90C-5D22A2BF257F}"

[Setup]
AppId={{#MyAppGuid}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppName}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes

OutputDir=.\output
OutputBaseFilename={#MyAppName}-win-x64
SetupIconFile=..\src-tauri\icons\icon.ico

; 压缩配置：显式 lzma2/max + 4 线程并行压缩大文件（bin\codex.exe 约 284 MB），
; 保持压缩率的同时大幅缩短编译耗时；SolidCompression 保持默认 no（逐文件压缩）。
Compression=lzma2/max
LZMANumBlockThreads=4

WizardStyle=modern
PrivilegesRequired=lowest
DisableDirPage=false
UsePreviousAppDir=true
CloseApplications=yes

; 会提示用户重启
RestartIfNeededByRun=no

; Windows 10/11（README 要求）；exe 为 AMD64，仅允许 64 位系统（x64/arm64，即 x64compatible 语义）
MinVersion=10.0
ArchitecturesAllowed=x64 arm64
ArchitecturesInstallIn64BitMode=x64 arm64

[Languages]
Name: chinese; MessagesFile: compiler:Languages\ChineseSimplified.isl

[Components]
Name: openai_bundled; Description: openai-bundled 内置插件市场（browser / computer-use / visualize 等）
Name: openai_primary_runtime; Description: openai-primary-runtime 插件市场（documents / pdf / spreadsheets / presentations 等，含完整运行时）

[Files]
Source: .\codex-ui.exe; DestDir: {app}; Flags: ignoreversion overwritereadonly replacesameversion
Source: .\bin\*; DestDir: {app}\bin; Flags: recursesubdirs ignoreversion overwritereadonly replacesameversion
; 内置插件市场所需的 codex primary-runtime（含完整 dependencies），复制到 codex 缓存位置；
; 按文件判断：目标已存在则跳过、缺失才补齐（onlyifdoesntexist），断电重装可自愈同版本残缺；
; 卸载 codex-ui 时保留该 codex 资源。
Source: .\codex-runtimes\codex-primary-runtime\*; DestDir: {code:CodexUserProfile}\.cache\codex-runtimes\codex-primary-runtime; Flags: recursesubdirs uninsneveruninstall onlyifdoesntexist; Components: openai_primary_runtime
; openai-bundled 复制到 codex 认可的 bundled-marketplaces 位置；按文件判断：已存在跳过、缺失补齐；
; 卸载 codex-ui 时保留该 codex 资源。
Source: .\bundled-marketplaces\openai-bundled\*; DestDir: {code:CodexHome}\.tmp\bundled-marketplaces\openai-bundled; Flags: recursesubdirs uninsneveruninstall onlyifdoesntexist; Components: openai_bundled

[Tasks]
Name: desktopicon; Description: {cm:CreateDesktopIcon}

[Icons]
Name: {group}\{#MyAppName} 卸载; Filename: {uninstallexe}
Name: {group}\{#MyAppName}; Filename: {app}\{#MyAppExeName}; WorkingDir: {app}
Name: {autodesktop}\{#MyAppName}; Filename: {app}\{#MyAppExeName}; Tasks: desktopicon

[Run]
Filename: {app}\{#MyAppExeName}; WorkingDir: {app}; Description: 运行 {#MyAppName}; Flags: postinstall nowait skipifsilent

[Code]

// 当前用户主目录（%USERPROFILE%），与 codex 判定 primary-runtime 缓存位置一致。
function CodexUserProfile(S: String): String;
begin
  Result := GetEnv('USERPROFILE');
end;

// codex home：优先 CODEX_HOME，否则 %USERPROFILE%\.codex（与应用 codex_home() 一致）。
function CodexHome(S: String): String;
var
  ch: String;
begin
  ch := GetEnv('CODEX_HOME');
  if ch <> '' then
    Result := ch
  else
    Result := GetEnv('USERPROFILE') + '\.codex';
end;

// 执行卸载（升级时先静默卸载旧版本）
procedure UnInstall();
var
  ResultCode: Integer;
  UnInstallPath: String;
begin
  // 当前用户安装：卸载注册表在 HKCU；管理员安装：在 HKLM（回退查找）
  if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#MyAppGuid}_is1', 'UninstallString', UnInstallPath) then
  begin
    UnInstallPath := RemoveQuotes(UnInstallPath);
    Exec(UnInstallPath, '/silent', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  end
  else if RegQueryStringValue(HKLM, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#MyAppGuid}_is1', 'UninstallString', UnInstallPath) then
  begin
    UnInstallPath := RemoveQuotes(UnInstallPath);
    Exec(UnInstallPath, '/silent', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  end;
end;

// 下一步 点击事件（wpReady = 准备安装）
function NextButtonClick(CurPageID: Integer): Boolean;
begin
  if CurPageID = wpReady then
  begin
    UnInstall();
  end;
  Result := true;
end;
