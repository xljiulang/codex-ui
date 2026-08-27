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

[Files]
Source: .\codex-ui.exe; DestDir: {app}; Flags: ignoreversion overwritereadonly replacesameversion
Source: .\bin\*; DestDir: {app}\bin; Flags: recursesubdirs ignoreversion overwritereadonly replacesameversion
; 随包只放 openai-bundled 插件市场（.tar.gz）；codex-primary-runtime 不再进安装包，
; 由 codex-ui 启动后按需下载/升级到 canonical 位置；卸载 codex-ui 时保留该 codex 资源。
Source: .\marketplaces\openai-bundled.tar.gz; DestDir: {app}\marketplaces; Flags: ignoreversion overwritereadonly replacesameversion
; 微信接入 sidecar（wechat-channel 的 self-contained ESM bundle，单文件）；
; 布局对应 Rust 解析器候选 <exe>\resources\wechat-sidecar\wechat-sidecar.mjs。
Source: .\resources\wechat-sidecar\*; DestDir: {app}\resources\wechat-sidecar; Flags: recursesubdirs ignoreversion overwritereadonly replacesameversion

[Tasks]
Name: desktopicon; Description: {cm:CreateDesktopIcon}

[Icons]
Name: {group}\{#MyAppName} 卸载; Filename: {uninstallexe}
Name: {group}\{#MyAppName}; Filename: {app}\{#MyAppExeName}; WorkingDir: {app}
Name: {autodesktop}\{#MyAppName}; Filename: {app}\{#MyAppExeName}; Tasks: desktopicon

[Run]
Filename: {app}\{#MyAppExeName}; WorkingDir: {app}; Description: 运行 {#MyAppName}; Flags: postinstall nowait skipifsilent

[Code]

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
