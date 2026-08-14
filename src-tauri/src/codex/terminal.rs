use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::codex::path_util::clean_path;

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

/// 校验 cwd 为存在的绝对目录（与 session_fs 的 resolve_root 语义一致）
fn validate_cwd(cwd: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(cwd);
    if !p.is_absolute() {
        return Err("工作目录必须为绝对路径".into());
    }
    let meta = std::fs::metadata(&p).map_err(|e| format!("无法访问目录 {}: {e}", clean_path(&p)))?;
    if !meta.is_dir() {
        return Err(format!("不是目录: {}", clean_path(&p)));
    }
    Ok(p)
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

/// 创建新终端：以给定目录为 cwd 启动 PowerShell（ConPTY），并启动 reader 线程转发输出。
/// id 由前端生成（terminal:<uuid>），保证标签与后端会话一一对应，天然支持多开。
#[tauri::command]
pub fn terminal_spawn(
    app: AppHandle,
    state: State<'_, TerminalState>,
    id: String,
    cwd: String,
) -> Result<TerminalSpawnResult, String> {
    if id.trim().is_empty() {
        return Err("终端 id 不能为空".into());
    }
    let dir = validate_cwd(&cwd)?;
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

    let mut cmd = CommandBuilder::new("powershell.exe");
    cmd.args(["-NoLogo", "-NoExit", "-Command", PS_STARTUP]);
    cmd.cwd(&dir);
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("启动 PowerShell 失败: {e}"))?;
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
    use std::time::{Duration, Instant};

    fn state() -> TerminalState {
        TerminalState(Mutex::new(HashMap::new()))
    }

    #[test]
    fn kill_unknown_id_is_idempotent() {
        let s = state();
        assert!(kill_session(&s, "nope").is_ok());
    }

    #[test]
    fn spawn_rejects_invalid_cwd() {
        assert!(validate_cwd("relative/path").is_err());
        let missing = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("no_such_dir_for_terminal_test");
        assert!(validate_cwd(missing.to_str().unwrap()).is_err());
    }

    #[test]
    fn spawn_accepts_existing_dir() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert!(validate_cwd(dir.to_str().unwrap()).is_ok());
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
