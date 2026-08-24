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
; 两个 codex 插件市场以 tar.xz 打包进安装包（已 xz 压缩，仅 160MB/17MB 供 Inno 压缩，构建快），安装时由自带 tar.exe 解压；
; 解压带 -k：目标文件已存在则跳过、不覆盖；卸载 codex-ui 时保留该 codex 资源。
Source: .\Components\codex-primary-runtime.tar.xz; DestDir: {tmp}
; openai-bundled 市场包；同上，解压到 bundled-marketplaces 位置。
Source: .\Components\openai-bundled.tar.xz; DestDir: {tmp}
; 自包含 tar.exe（用于安装时解压两个包），始终随包提供。
Source: .\Components\tar.exe; DestDir: {tmp}

[Tasks]
Name: desktopicon; Description: {cm:CreateDesktopIcon}

[Icons]
Name: {group}\{#MyAppName} 卸载; Filename: {uninstallexe}
Name: {group}\{#MyAppName}; Filename: {app}\{#MyAppExeName}; WorkingDir: {app}
Name: {autodesktop}\{#MyAppName}; Filename: {app}\{#MyAppExeName}; Tasks: desktopicon

[Run]
Filename: {tmp}\tar.exe; Parameters: {code:TarRuntimeParams}; StatusMsg: 正在解压 openai-primary-runtime 插件市场...; Flags: runhidden waituntilterminated; Check: ShouldExtractRuntime
Filename: {tmp}\tar.exe; Parameters: {code:TarBundledParams}; StatusMsg: 正在解压 openai-bundled 插件市场...; Flags: runhidden waituntilterminated; Check: ShouldExtractBundled
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

// 目标 primary-runtime 是否已完整存在（以 runtime.json 为完成标记）。
function ShouldExtractRuntime(): Boolean;
begin
  Result := not FileExists(CodexUserProfile('') + '\.cache\codex-runtimes\codex-primary-runtime\runtime.json');
end;

// openai-bundled 是否已物化（以 .materialization-key 为完成标记）。
function ShouldExtractBundled(): Boolean;
begin
  Result := not FileExists(CodexHome('') + '\.tmp\bundled-marketplaces\openai-bundled\.materialization-key');
end;

// 供 [Run] Parameters 使用：{code:} 调用需带一个 String 参数；返回带引号的完整参数串（运行时展开）。
function TarRuntimeParams(S: String): String;
begin
  Result := '-xf "' + ExpandConstant('{tmp}\codex-primary-runtime.tar.xz') + '" -k -C "' + CodexUserProfile('') + '\.cache\codex-runtimes"';
end;

function TarBundledParams(S: String): String;
begin
  Result := '-xf "' + ExpandConstant('{tmp}\openai-bundled.tar.xz') + '" -k -C "' + CodexHome('') + '\.tmp\bundled-marketplaces"';
end;

// 预建解压目标目录（tar -C 要求目录已存在）。
procedure EnsureExtractionDirs();
begin
  ForceDirectories(CodexUserProfile('') + '\.cache\codex-runtimes');
  ForceDirectories(CodexHome('') + '\.tmp\bundled-marketplaces');
end;

// 安装开始前预建解压目录；解压本身由 [Run] 条目执行（保持向导响应并显示进度）。
procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
    EnsureExtractionDirs();
end;
