use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::codex::app_server::{app_exe_dir, prepend_bin_path};
use crate::codex::settings;
use crate::codex::util::resolve_workspace_dir;

/// 终端会话：持有 ConPTY 主端、写端与子进程。
/// reader 线程在输出 EOF（进程退出或被 kill）后自行清理 map 条目。
pub(crate) struct PtySession {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Option<Box<dyn Child + Send + Sync>>>,
}

/// 终端会话表：id -> 会话；由 setup 注入 app state，应用退出时随 drop 清理全部 PTY。
pub struct TerminalState(pub(crate) Mutex<HashMap<String, Arc<PtySession>>>);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSpawnResult {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputPayload {
    pub id: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalExitPayload {
    pub id: String,
    pub exit_code: i32,
}

/// 默认尺寸：前端挂载后会按 fit 结果立即 resize
const DEFAULT_ROWS: u16 = 30;
const DEFAULT_COLS: u16 = 100;

/// PowerShell 启动参数：-NoLogo 去横幅；启动时把控制台编码固定为 UTF-8，
/// 保证中文/Unicode 输出经 ConPTY 读回后不乱码（zh-CN 默认 OEM 936）。
/// 同时覆写 prompt：每次回到提示符先输出隐藏标记 OSC 133;D（BEL 终止），
/// 前端据此判定“命令已执行完、回到空闲”，提示符外观保持默认 `PS <路径> `。
/// 注：Windows PowerShell 5.1 不支持反引号 `e 转义，控制字符用 [char] 构造。
/// 另：2025-12 安全更新（CVE-2025-54100）起 Invoke-WebRequest 解析 HTML 前会弹
/// “脚本执行风险”确认；这里设置会话级默认参数等效自动带 -UseBasicParsing，
/// wget/iwr 不再弹窗（代价是 Links/Images/Forms 等 DOM 解析属性不可用，
/// 个别场景可显式 -UseBasicParsing:$false 恢复完整解析）。
const PS_STARTUP: &str = "chcp 65001 > $null; [Console]::InputEncoding = [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $PSDefaultParameterValues['Invoke-WebRequest:UseBasicParsing'] = $true; function prompt { \"$([char]27)]133;D$([char]7)PS $($executionContext.SessionState.Path.CurrentLocation)> \" }";

/// cmd 启动参数：/D 跳过注册表 AutoRun，/K 执行启动串后保持交互；
/// 启动时把活动代码页固定为 UTF-8（chcp 65001），保证中文/Unicode
/// 输出经 ConPTY 读回后不乱码（zh-CN 默认 OEM 936）。
/// 同时覆写 prompt：每次回到提示符先输出隐藏标记 OSC 133;D（ST 终止，
/// `$E` 即 ESC、`$E\` 即 ST），前端据此判定“命令已执行完、回到空闲”，
/// 提示符外观保持默认 `C:\路径> `。
/// 注：cmd 的版权横幅无法用命令行参数去除，仅装饰性输出，不影响标记判定。
const CMD_STARTUP: &str = "chcp 65001 >nul & prompt $E]133;D$E\\$P$G ";

/// 解析安装目录名的前导数字版本号（如 8 → 8、7-preview → 7）；无前导数字返回 None
fn dir_major_version(name: &str) -> Option<u64> {
    let digits: String = name
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

/// 扫描 %ProgramFiles%\PowerShell\*（根目录按传入顺序）下的 pwsh.exe，
/// 按目录名前导数字版本号取最新；同版本号取修改时间更新者；无命中返回 None。
fn find_pwsh(program_files_roots: &[PathBuf]) -> Option<PathBuf> {
    let mut best: Option<(u64, std::time::SystemTime, PathBuf)> = None;
    for root in program_files_roots {
        let powershell_dir = root.join("PowerShell");
        let Ok(entries) = std::fs::read_dir(&powershell_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir_path = entry.path();
            if !dir_path.is_dir() {
                continue;
            }
            let Some(version) = entry.file_name().to_str().and_then(dir_major_version) else {
                continue;
            };
            let exe = dir_path.join("pwsh.exe");
            if !exe.is_file() {
                continue;
            }
            let mtime = std::fs::metadata(&exe)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            let replace = match &best {
                None => true,
                Some((best_version, best_mtime, _)) => {
                    version > *best_version || (version == *best_version && mtime > *best_mtime)
                }
            };
            if replace {
                best = Some((version, mtime, exe));
            }
        }
    }
    best.map(|(_, _, exe)| exe)
}

/// 解析 PowerShell 可执行文件：仅扫描 %ProgramW6432%/%ProgramFiles% 下的
/// PowerShell\* 目录取最新 pwsh.exe；找不到直接回退系统自带 powershell.exe（5.1），
/// 不探测 PATH / 商店别名。每次调用实时解析，不缓存。
fn resolve_powershell() -> String {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("ProgramW6432") {
        roots.push(PathBuf::from(p));
    }
    if let Ok(p) = std::env::var("ProgramFiles") {
        roots.push(PathBuf::from(p));
    }
    match find_pwsh(&roots) {
        Some(exe) => exe.to_string_lossy().into_owned(),
        None => "powershell.exe".into(),
    }
}

/// 按设置选择终端 Shell：powershell → PowerShell（优先 pwsh 7+，缺失回退 5.1），
/// 其余（含未知/缺失）→ cmd
fn shell_command(shell: &str) -> (String, Vec<String>) {
    if shell == "powershell" {
        (
            resolve_powershell(),
            vec![
                "-NoLogo".into(),
                "-NoExit".into(),
                "-Command".into(),
                PS_STARTUP.into(),
            ],
        )
    } else {
        (
            "cmd.exe".into(),
            vec!["/D".into(), "/K".into(), CMD_STARTUP.into()],
        )
    }
}

/// 结束并移除指定会话；会话不存在时视为已清理（幂等）
fn kill_session(state: &TerminalState, id: &str) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    let Some(session) = guard.remove(id) else {
        return Ok(());
    };
    if let Ok(mut child_guard) = session.child.lock() {
        if let Some(child) = child_guard.as_mut() {
            let _ = child.kill();
        }
    }
    Ok(())
}

/// 终止并清空全部终端会话：应用退出时收尾，避免 ConPTY 子进程（cmd/powershell）孤儿残留。
/// 与 kill_session 同一终止逻辑；遍历全部，drain 后 map 清空，可重复调用（重复杀安全）。
pub fn kill_all(state: &TerminalState) {
    let mut guard = match state.0.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    for (_, session) in guard.drain() {
        if let Ok(mut child_guard) = session.child.lock() {
            if let Some(child) = child_guard.as_mut() {
                let _ = child.kill();
            }
        }
    }
}

/// 创建新终端：以给定工作区按设置启动 cmd 或 PowerShell（ConPTY），
/// 并启动 reader 线程转发输出。
/// id 由前端生成（terminal:<uuid>），保证标签与后端会话一一对应，天然支持多开。
#[tauri::command]
pub fn terminal_spawn(
    app: AppHandle,
    state: State<'_, TerminalState>,
    id: String,
    workspace: String,
) -> Result<TerminalSpawnResult, String> {
    if id.trim().is_empty() {
        return Err("终端 id 不能为空".into());
    }
    let dir = resolve_workspace_dir(&workspace)?;
    {
        let guard = state.0.lock().map_err(|e| e.to_string())?;
        if guard.contains_key(&id) {
            return Err(format!("终端已存在: {id}"));
        }
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("创建终端失败: {e}"))?;

    // 与前端标签标题读取同一份 settings.json（settings_set 已先落盘），保证一致
    let shell = app
        .path()
        .app_data_dir()
        .map(|dir| settings::load(&dir).terminal_shell)
        .unwrap_or_else(|_| "cmd".into());
    let (program, args) = shell_command(&shell);
    let mut cmd = CommandBuilder::new(&program);
    cmd.args(&args);
    cmd.cwd(&dir);
    // 与应用启动 codex 一致：把应用自身目录的 bin 前插到终端 PATH，
    // 使终端里可直接调用 bin 目录中的 CLI 工具（fd/rg/sg 等）。
    let existing_path = std::env::var("PATH").unwrap_or_default();
    if let Some(app_dir) = app_exe_dir() {
        if let Some(joined) = prepend_bin_path(&existing_path, &app_dir) {
            cmd.env("PATH", joined);
        }
    }
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("启动终端失败: {e}"))?;
    drop(pair.slave);

    let master: Box<dyn MasterPty + Send> = pair.master;
    let writer = master
        .take_writer()
        .map_err(|e| format!("获取终端写端失败: {e}"))?;
    let mut reader = master
        .try_clone_reader()
        .map_err(|e| format!("获取终端读端失败: {e}"))?;

    let session = Arc::new(PtySession {
        master: Arc::new(Mutex::new(master)),
        writer: Mutex::new(writer),
        child: Mutex::new(Some(child)),
    });
    {
        let mut guard = state.0.lock().map_err(|e| e.to_string())?;
        guard.insert(id.clone(), session);
    }

    // reader 线程：分块读取输出 emit "terminal/output"；EOF 后取退出码 emit
    // "terminal/exit" 并清理 map（kill 路径下 kill_session 已先移除，此处幂等）。
    let app_for_thread = app.clone();
    drop(app);
    let id_for_thread = id.clone();
    std::thread::spawn(move || {
        let state_for_thread = app_for_thread.state::<TerminalState>();
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let _ = app_for_thread.emit(
                        "terminal/output",
                        TerminalOutputPayload {
                            id: id_for_thread.clone(),
                            data,
                        },
                    );
                }
                Err(_) => break,
            }
        }
        let exit_code = {
            let mut code = -1;
            if let Ok(guard) = state_for_thread.0.lock() {
                if let Some(session) = guard.get(&id_for_thread) {
                    if let Ok(mut child_guard) = session.child.lock() {
                        if let Some(child) = child_guard.as_mut() {
                            if let Ok(status) = child.wait() {
                                code = status.exit_code() as i32;
                            }
                        }
                    }
                }
            }
            code
        };
        let _ = app_for_thread.emit(
            "terminal/exit",
            TerminalExitPayload {
                id: id_for_thread.clone(),
                exit_code,
            },
        );
        let lock_result = state_for_thread.0.lock();
        if let Ok(mut guard) = lock_result {
            guard.remove(&id_for_thread);
        }
    });

    Ok(TerminalSpawnResult { id })
}

/// 向终端写入输入（xterm onData → 原始字节）
#[tauri::command]
pub fn terminal_write(
    state: State<'_, TerminalState>,
    id: String,
    data: String,
) -> Result<(), String> {
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    let session = guard
        .get(&id)
        .ok_or_else(|| format!("终端不存在: {id}"))?;
    let mut writer = session.writer.lock().map_err(|e| e.to_string())?;
    writer
        .write_all(data.as_bytes())
        .and_then(|_| writer.flush())
        .map_err(|e| format!("写入终端失败: {e}"))
}

/// 调整终端尺寸（xterm fit 后同步 ConPTY 行列数）
#[tauri::command]
pub fn terminal_resize(
    state: State<'_, TerminalState>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    let session = guard
        .get(&id)
        .ok_or_else(|| format!("终端不存在: {id}"))?;
    let master = session.master.lock().map_err(|e| e.to_string())?;
    master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("调整终端尺寸失败: {e}"))
}

/// 结束终端进程并移除会话（幂等：会话不存在时返回 Ok）
#[tauri::command]
pub fn terminal_kill(state: State<'_, TerminalState>, id: String) -> Result<(), String> {
    kill_session(&state, &id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex as StdMutex;
    use std::time::{Duration, Instant};

    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    /// 串行设置/恢复多个环境变量（避免并行测试互相干扰；Mutex 不可重入，勿嵌套）
    fn with_envs(pairs: &[(&str, Option<&str>)], f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap();
        let old: Vec<(&str, Option<std::ffi::OsString>)> = pairs
            .iter()
            .map(|(k, _)| (*k, std::env::var_os(k)))
            .collect();
        for (k, v) in pairs {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
        f();
        for (k, v) in old {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }

    /// 在根目录下伪造 PowerShell\<版本>\pwsh.exe 并返回其路径
    fn write_pwsh(root: &Path, version_dir: &str) -> PathBuf {
        let exe = root
            .join("PowerShell")
            .join(version_dir)
            .join("pwsh.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"MZ").unwrap();
        exe
    }

    fn set_modified(p: &Path, t: std::time::SystemTime) {
        let f = std::fs::File::options().write(true).open(p).unwrap();
        f.set_times(std::fs::FileTimes::new().set_modified(t))
            .unwrap();
    }

    fn state() -> TerminalState {
        TerminalState(Mutex::new(HashMap::new()))
    }

    #[test]
    fn kill_unknown_id_is_idempotent() {
        let s = state();
        assert!(kill_session(&s, "nope").is_ok());
    }

    #[test]
    fn kill_all_empty_is_noop() {
        let s = state();
        kill_all(&s);
        assert!(s.0.lock().unwrap().is_empty());
    }

    /// kill_all 应清空全部会话并终止各子进程（真实 ConPTY 冒烟，幂等可重复）。
    #[test]
    fn kill_all_drains_all_sessions() {
        let s = state();
        for i in 0..2 {
            let pty_system = native_pty_system();
            let pair = pty_system
                .openpty(PtySize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .expect("openpty");
            let mut cmd = CommandBuilder::new("cmd.exe");
            cmd.args(["/D", "/K", CMD_STARTUP]);
            cmd.cwd(std::env::current_dir().unwrap());
            let child = pair.slave.spawn_command(cmd).expect("spawn cmd");
            drop(pair.slave);
            let master: Box<dyn MasterPty + Send> = pair.master;
            let writer = master.take_writer().expect("take writer");
            let session = Arc::new(PtySession {
                master: Arc::new(Mutex::new(master)),
                writer: Mutex::new(writer),
                child: Mutex::new(Some(child)),
            });
            s.0.lock().unwrap().insert(format!("term-{i}"), session);
        }
        assert_eq!(s.0.lock().unwrap().len(), 2);
        kill_all(&s);
        assert!(s.0.lock().unwrap().is_empty());
        // 幂等：再次调用无副作用、不报错
        kill_all(&s);
    }

    #[test]
    fn spawn_rejects_invalid_cwd() {
        assert!(resolve_workspace_dir("relative/path").is_err());
        let missing = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("no_such_dir_for_terminal_test");
        assert!(resolve_workspace_dir(missing.to_str().unwrap()).is_err());
    }

    #[test]
    fn spawn_accepts_existing_dir() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert!(resolve_workspace_dir(dir.to_str().unwrap()).is_ok());
    }

    #[test]
    fn shell_command_maps_powershell_and_cmd() {
        let (prog, args) = shell_command("powershell");
        assert_eq!(prog, resolve_powershell());
        assert_eq!(
            args,
            vec!["-NoLogo", "-NoExit", "-Command", PS_STARTUP]
        );

        let (prog, args) = shell_command("cmd");
        assert_eq!(prog, "cmd.exe");
        assert_eq!(args, vec!["/D", "/K", CMD_STARTUP]);

        // 未知/缺失值统一回落 cmd
        let (prog, args) = shell_command("bogus");
        assert_eq!(prog, "cmd.exe");
        assert_eq!(args, vec!["/D", "/K", CMD_STARTUP]);
    }

    #[test]
    fn find_pwsh_picks_highest_version_dir() {
        let root = tempfile::tempdir().unwrap();
        let v7 = write_pwsh(root.path(), "7");
        let v8 = write_pwsh(root.path(), "8");
        assert_eq!(find_pwsh(&[root.path().to_path_buf()]).unwrap(), v8);
        assert_ne!(v7, v8);
    }

    #[test]
    fn find_pwsh_same_major_uses_newer_mtime() {
        let root = tempfile::tempdir().unwrap();
        let stable = write_pwsh(root.path(), "7");
        let preview = write_pwsh(root.path(), "7-preview");
        set_modified(&stable, std::time::UNIX_EPOCH);
        set_modified(
            &preview,
            std::time::UNIX_EPOCH + Duration::from_secs(1),
        );
        assert_eq!(
            find_pwsh(&[root.path().to_path_buf()]).unwrap(),
            preview
        );
    }

    #[test]
    fn find_pwsh_prefers_first_root_on_tie() {
        let root_a = tempfile::tempdir().unwrap();
        let root_b = tempfile::tempdir().unwrap();
        let a = write_pwsh(root_a.path(), "7");
        let b = write_pwsh(root_b.path(), "7");
        set_modified(&a, std::time::UNIX_EPOCH);
        set_modified(&b, std::time::UNIX_EPOCH);
        assert_eq!(
            find_pwsh(&[
                root_a.path().to_path_buf(),
                root_b.path().to_path_buf(),
            ])
            .unwrap(),
            a
        );
    }

    #[test]
    fn find_pwsh_missing_returns_none() {
        let root = tempfile::tempdir().unwrap();
        // 无 PowerShell 目录
        assert_eq!(find_pwsh(&[root.path().to_path_buf()]), None);
        // PowerShell 目录存在但版本目录无前导数字 / 无 pwsh.exe
        std::fs::create_dir_all(root.path().join("PowerShell").join("foo")).unwrap();
        assert_eq!(find_pwsh(&[root.path().to_path_buf()]), None);
    }

    #[test]
    fn resolve_powershell_uses_found_pwsh() {
        let root = tempfile::tempdir().unwrap();
        let exe = write_pwsh(root.path(), "8");
        let root_s = root.path().to_string_lossy().into_owned();
        with_envs(
            &[("ProgramW6432", Some(&root_s)), ("ProgramFiles", None)],
            || {
                assert_eq!(
                    resolve_powershell(),
                    exe.to_string_lossy().into_owned()
                );
            },
        );
    }

    #[test]
    fn resolve_powershell_falls_back_when_no_pwsh() {
        let empty = tempfile::tempdir().unwrap();
        let empty_s = empty.path().to_string_lossy().into_owned();
        with_envs(
            &[
                ("ProgramW6432", Some(&empty_s)),
                ("ProgramFiles", Some(&empty_s)),
            ],
            || {
                assert_eq!(resolve_powershell(), "powershell.exe");
            },
        );
    }

    /// 真实 ConPTY 冒烟：以 cmd + UTF-8 启动参数拉起子进程，
    /// 宿主应答光标位置查询（xterm 同款行为）后写入命令，
    /// 验证输入回显、UTF-8 活动代码页与提示符空闲标记（OSC 133;D）。
    #[test]
    fn pty_smoke_cmd_output() {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(["/D", "/K", CMD_STARTUP]);
        cmd.cwd(std::env::current_dir().unwrap());
        let mut child = pair.slave.spawn_command(cmd).expect("spawn cmd");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("clone reader");
        let mut writer = pair.master.take_writer().expect("take writer");
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
                    }
                    Err(_) => break,
                }
            }
        });

        std::thread::sleep(Duration::from_millis(600));
        writer
            .write_all(
                b"echo PTY-OK-42\r\necho \xe7\xbb\x88\xe7\xab\xaf\xe6\xb5\x8b\xe8\xaf\x95\r\n",
            )
            .expect("write input");
        writer.flush().expect("flush input");

        let mut out = String::new();
        let mut responded_dsr = false;
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(chunk) => {
                    out.push_str(&chunk);
                    // ConPTY（INHERIT_CURSOR）启动先查询光标位置，宿主须应答
                    if !responded_dsr && out.contains("\x1b[6n") {
                        responded_dsr = true;
                        let _ = writer.write_all(b"\x1b[24;1R");
                        let _ = writer.flush();
                    }
                    if out.contains("PTY-OK-42")
                        && out.contains("终端测试")
                        && out.contains("65001")
                    {
                        break;
                    }
                }
                Err(_) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        break;
                    }
                }
            }
        }
        let _ = child.kill();
        assert!(
            out.contains("PTY-OK-42") && out.contains("终端测试"),
            "pty output: {out}"
        );
        assert!(
            out.contains("\x1b]133;D"),
            "pty output missing prompt marker: {out}"
        );
        assert!(
            out.contains("65001"),
            "pty output missing active code page 65001: {out}"
        );
    }

    /// 真实 ConPTY 冒烟：以 PowerShell + UTF-8 启动参数拉起子进程，
    /// 宿主应答光标位置查询（xterm 同款行为）后写入命令，
    /// 验证输入回显与运行时构造的中文输出均能 UTF-8 无损往返。
    #[test]
    fn pty_smoke_powershell_output() {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");
        let mut cmd = CommandBuilder::new("powershell.exe");
        cmd.args(["-NoLogo", "-NoExit", "-Command", PS_STARTUP]);
        cmd.cwd(std::env::current_dir().unwrap());
        let mut child = pair.slave.spawn_command(cmd).expect("spawn powershell");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("clone reader");
        let mut writer = pair.master.take_writer().expect("take writer");
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
                    }
                    Err(_) => break,
                }
            }
        });

        std::thread::sleep(Duration::from_millis(600));
        // 中文由码点运行时构造，避免仅凭输入回显误判编码正确
        writer
            .write_all(
                b"Write-Output 'PTY-OK-42' ([string]::Join('', [char[]](0x7EC8,0x7AEF,0x6D4B,0x8BD5)))\r\nWrite-Output ('IWR-DEFAULT=' + $PSDefaultParameterValues['Invoke-WebRequest:UseBasicParsing'])\r\n",
            )
            .expect("write input");
        writer.flush().expect("flush input");

        let mut out = String::new();
        let mut responded_dsr = false;
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(chunk) => {
                    out.push_str(&chunk);
                    // ConPTY（INHERIT_CURSOR）启动先查询光标位置，宿主须应答
                    if !responded_dsr && out.contains("\x1b[6n") {
                        responded_dsr = true;
                        let _ = writer.write_all(b"\x1b[24;1R");
                        let _ = writer.flush();
                    }
                    if out.contains("PTY-OK-42")
                        && out.contains("终端测试")
                        && out.contains("IWR-DEFAULT=True")
                    {
                        break;
                    }
                }
                Err(_) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        break;
                    }
                }
            }
        }
        let _ = child.kill();
        assert!(
            out.contains("PTY-OK-42") && out.contains("终端测试"),
            "pty output: {out}"
        );
        assert!(
            out.contains("\x1b]133;D"),
            "pty output missing prompt marker: {out}"
        );
        assert!(
            out.contains("IWR-DEFAULT=True"),
            "pty output missing Invoke-WebRequest UseBasicParsing default: {out}"
        );
    }

}
