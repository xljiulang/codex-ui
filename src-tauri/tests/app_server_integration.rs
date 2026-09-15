//! 集成冒烟测试：直接驱动真实 `codex app-server --stdio`，验证本项目依赖的协议流程。
//! 设置环境变量 CODEX_BIN 指向 codex 可执行文件时才会运行，否则跳过。

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// 解析 CODEX_BIN 指向的真实 codex 可执行文件（可能带路径或只是命令名）。
/// npm 安装的 `codex` 在 PATH 里通常只是 `.cmd`/`.ps1` shim，Rust 的
/// `Command::new` 无法直接启动；这里按 npm 嵌套/扁平布局解析出真实 codex.exe，
/// 找不到再回退官方安装目录，最后才原样返回（spawn 时报错而非静默跳过）。
fn codex_bin() -> Option<String> {
    let raw = std::env::var("CODEX_BIN").ok()?;
    Some(resolve_codex_bin(&raw))
}

fn resolve_codex_bin(raw: &str) -> String {
    let direct = PathBuf::from(raw);
    let lower = raw.to_ascii_lowercase();
    if direct.is_file() && lower.ends_with(".exe") {
        return raw.to_string();
    }
    // 直接指向 shim 文件（.cmd/.bat/.ps1）：在其目录下按 npm 布局解析
    if direct.is_file()
        && (lower.ends_with(".cmd") || lower.ends_with(".bat") || lower.ends_with(".ps1"))
    {
        if let Some(exe) = direct.parent().and_then(npm_codex_exe) {
            return exe.to_string_lossy().into_owned();
        }
    }
    // 裸命令名：PATH 里先找原生 exe，再找 shim 并按 npm 布局解析
    if !direct.is_absolute() {
        if let Some(p) = find_on_path(&format!("{raw}.exe")) {
            return p;
        }
        let shim = find_on_path(&format!("{raw}.cmd"))
            .or_else(|| find_on_path(&format!("{raw}.bat")))
            .or_else(|| find_on_path(&format!("{raw}.ps1")));
        if let Some(shim) = shim {
            if let Some(exe) = Path::new(&shim).parent().and_then(npm_codex_exe) {
                return exe.to_string_lossy().into_owned();
            }
        }
    }
    // 官方安装兜底：%LOCALAPPDATA%\OpenAI\Codex\bin\*\codex.exe 取最新
    if let Some(exe) = official_codex_exe() {
        return exe.to_string_lossy().into_owned();
    }
    raw.to_string()
}

fn find_on_path(name: &str) -> Option<String> {
    let path = std::env::var("PATH").ok()?;
    for root in std::env::split_paths(&path) {
        let p = root.join(name);
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

/// npm 前缀目录下按嵌套/扁平布局找 @openai 平台包里的真实 codex.exe（x64/arm64）
fn npm_codex_exe(prefix: &Path) -> Option<PathBuf> {
    const LAYOUTS: [&[&str]; 2] = [
        &["node_modules", "@openai", "codex", "node_modules", "@openai"],
        &["node_modules", "@openai"],
    ];
    const TARGETS: [&[&str]; 2] = [
        &[
            "codex-win32-x64",
            "vendor",
            "x86_64-pc-windows-msvc",
            "bin",
            "codex.exe",
        ],
        &[
            "codex-win32-arm64",
            "vendor",
            "aarch64-pc-windows-msvc",
            "bin",
            "codex.exe",
        ],
    ];
    for layout in LAYOUTS {
        for target in TARGETS {
            let mut p = PathBuf::from(prefix);
            for seg in layout.iter().chain(target.iter()) {
                p.push(seg);
            }
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// 官方安装：glob %LOCALAPPDATA%\OpenAI\Codex\bin\*\codex.exe，按修改时间取最新
fn official_codex_exe() -> Option<PathBuf> {
    let root = std::env::var("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join("OpenAI").join("Codex").join("bin"))
        .ok()?;
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for entry in std::fs::read_dir(&root).ok()?.flatten() {
        let exe = entry.path().join("codex.exe");
        if exe.is_file() {
            let mtime = std::fs::metadata(&exe)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().map(|(_, t)| mtime > *t).unwrap_or(true) {
                best = Some((exe, mtime));
            }
        }
    }
    best.map(|(p, _)| p)
}

struct Server {
    child: Child,
    stdout: BufReader<std::process::ChildStdout>,
    stdin: std::process::ChildStdin,
    next_id: u64,
}

impl Server {
    fn start(cwd: &Path) -> Self {
        let bin = codex_bin().unwrap_or_else(|| "codex".into());
        let mut child = Command::new(bin)
            .args(["app-server", "--stdio"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn codex app-server");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let stdin = child.stdin.take().expect("stdin");
        Self {
            child,
            stdout,
            stdin,
            next_id: 1,
        }
    }

    fn send_raw(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").expect("write stdin");
    }

    fn request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.send_raw(&msg.to_string());
        id
    }

    fn notify(&mut self, method: &str) {
        self.send_raw(&json!({ "jsonrpc": "2.0", "method": method }).to_string());
    }

    /// 读取下一条消息，直到出现满足 predicate 的，返回该消息（跳过不匹配的）。
    fn wait_for<F>(&mut self, deadline: Instant, predicate: F) -> Option<Value>
    where
        F: Fn(&Value) -> bool,
    {
        let mut line = String::new();
        while Instant::now() < deadline {
            line.clear();
            match self.stdout.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                        if predicate(&v) {
                            return Some(v);
                        }
                    }
                }
                Err(_) => return None,
            }
        }
        None
    }
}

#[test]
fn app_server_handshake_list_start_turn() {
    if codex_bin().is_none() {
        eprintln!("跳过：未设置 CODEX_BIN");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");

    let mut server = Server::start(tmp.path());
    let deadline = Instant::now() + Duration::from_secs(90);

    let init_id = server.request(
        "initialize",
        json!({
            "clientInfo": { "name": "codex-ui-test", "title": "Codex UI Test", "version": "0.1.0" },
            "capabilities": { "experimentalApi": true, "requestAttestation": false }
        }),
    );
    let init_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(init_id))
        .expect("initialize response");
    assert!(init_resp.get("result").is_some(), "initialize 应成功: {init_resp}");
    server.notify("initialized");

    let list_id = server.request("thread/list", json!({ "limit": 5 }));
    let list_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(list_id))
        .expect("thread/list response");
    assert!(list_resp.get("result").is_some(), "thread/list 应成功: {list_resp}");

    let start_id = server.request(
        "thread/start",
        json!({
            "cwd": tmp.path().to_string_lossy(),
            "approvalPolicy": "untrusted",
            "sandbox": "read-only"
        }),
    );
    let start_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(start_id))
        .expect("thread/start response");
    let thread_id = start_resp["result"]["thread"]["id"]
        .as_str()
        .expect("thread id")
        .to_string();

    let turn_id = server.request(
        "turn/start",
        json!({
            "threadId": thread_id,
            "input": [{"type": "text", "text": "只回复 OK 两个字母", "text_elements": []}]
        }),
    );
    let turn_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(turn_id))
        .expect("turn/start response");
    assert!(turn_resp.get("result").is_some(), "turn/start 应成功: {turn_resp}");

    let completed = server
        .wait_for(deadline, |v| {
            v.get("method").and_then(|m| m.as_str()) == Some("turn/completed")
        })
        .expect("turn/completed");
    let status = completed["params"]["turn"]["status"].as_str().unwrap_or("");
    assert_eq!(status, "completed", "回合应完成: {completed}");

    let delete_id = server.request("thread/delete", json!({ "threadId": thread_id }));
    let _ = server.wait_for(deadline, |v| {
        v.get("id").and_then(|i| i.as_u64()) == Some(delete_id)
    });

    server.child.kill().ok();
    let _ = server.child.wait();
}

/// 稳定内置 Pinned 分区 id（与 codex 源码常量一致）。
const PINNED_SECTION_ID: &str = "01984de2-8f74-7c91-a3b2-5c5e937cf318";

/// 初始化握手并创建线程，返回 thread_id（新集成测试共用）。
fn start_thread(server: &mut Server, cwd: &Path, deadline: Instant) -> String {
    let init_id = server.request(
        "initialize",
        json!({
            "clientInfo": { "name": "codex-ui-test", "title": "Codex UI Test", "version": "0.1.0" },
            "capabilities": { "experimentalApi": true, "requestAttestation": false }
        }),
    );
    let init_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(init_id))
        .expect("initialize response");
    assert!(init_resp.get("result").is_some(), "initialize 应成功: {init_resp}");
    server.notify("initialized");

    let start_id = server.request(
        "thread/start",
        json!({
            "cwd": cwd.to_string_lossy(),
            "approvalPolicy": "untrusted",
            "sandbox": "read-only"
        }),
    );
    let start_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(start_id))
        .expect("thread/start response");
    start_resp["result"]["thread"]["id"]
        .as_str()
        .expect("thread id")
        .to_string()
}

/// 删除线程并结束 app-server（新集成测试共用）。
fn cleanup_thread(server: &mut Server, thread_id: &str, deadline: Instant) {
    let delete_id = server.request("thread/delete", json!({ "threadId": thread_id }));
    let _ = server.wait_for(deadline, |v| {
        v.get("id").and_then(|i| i.as_u64()) == Some(delete_id)
    });
    server.child.kill().ok();
    let _ = server.child.wait();
}

#[test]
fn app_server_goal_lifecycle() {
    if codex_bin().is_none() {
        eprintln!("跳过：未设置 CODEX_BIN");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut server = Server::start(tmp.path());
    let deadline = Instant::now() + Duration::from_secs(180);
    let thread_id = start_thread(&mut server, tmp.path(), deadline);

    // 挂载目标：thread/goal/set 返回 active
    let set_id = server.request(
        "thread/goal/set",
        json!({ "threadId": thread_id, "objective": "只回复 OK 两个字母，完成后停止" }),
    );
    let set_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(set_id))
        .expect("thread/goal/set response");
    let set_result = set_resp.get("result").expect("thread/goal/set 应成功: {set_resp}");
    assert_eq!(set_result["goal"]["status"].as_str().unwrap_or(""), "active");

    // 等待服务端 auto-continuation 完成后下发非 active 状态（协议值 complete）
    let updated = server
        .wait_for(deadline, |v| {
            v.get("method").and_then(|m| m.as_str()) == Some("thread/goal/updated")
                && v["params"]["goal"]["status"]
                    .as_str()
                    .is_some_and(|s| s != "active")
        })
        .expect("thread/goal/updated 非 active 状态");
    assert_eq!(
        updated["params"]["goal"]["status"].as_str().unwrap_or(""),
        "complete",
        "目标完成状态应为 complete: {updated}"
    );

    // goal/get 回读一致
    let get_id = server.request("thread/goal/get", json!({ "threadId": thread_id }));
    let get_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(get_id))
        .expect("thread/goal/get response");
    let goal = &get_resp["result"]["goal"];
    assert_eq!(goal["status"].as_str().unwrap_or(""), "complete");
    assert_eq!(goal["objective"].as_str().unwrap_or(""), "只回复 OK 两个字母，完成后停止");

    // goal/clear 清除并返回 cleared: true
    let clear_id = server.request("thread/goal/clear", json!({ "threadId": thread_id }));
    let clear_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(clear_id))
        .expect("thread/goal/clear response");
    assert_eq!(clear_resp["result"]["cleared"].as_bool(), Some(true));

    cleanup_thread(&mut server, &thread_id, deadline);
}

#[test]
fn app_server_memory_and_settings_update() {
    if codex_bin().is_none() {
        eprintln!("跳过：未设置 CODEX_BIN");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut server = Server::start(tmp.path());
    let deadline = Instant::now() + Duration::from_secs(90);
    let thread_id = start_thread(&mut server, tmp.path(), deadline);

    // 记忆模式开关（协议响应为空对象，断言无错误即成功）
    for mode in ["enabled", "disabled"] {
        let mem_id = server.request(
            "thread/memoryMode/set",
            json!({ "threadId": thread_id, "mode": mode }),
        );
        let mem_resp = server
            .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(mem_id))
            .expect("thread/memoryMode/set response");
        assert!(
            mem_resp.get("result").is_some(),
            "memoryMode/set({mode}) 应成功: {mem_resp}"
        );
    }

    // 线程设置更新：显式 null（恢复默认）与非空 effort 均被接受
    let s1_id = server.request(
        "thread/settings/update",
        json!({ "threadId": thread_id, "model": null, "effort": null }),
    );
    let s1_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(s1_id))
        .expect("thread/settings/update response 1");
    assert!(s1_resp.get("result").is_some(), "settings/update(nulls) 应成功: {s1_resp}");

    let s2_id = server.request(
        "thread/settings/update",
        json!({ "threadId": thread_id, "effort": "low" }),
    );
    let s2_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(s2_id))
        .expect("thread/settings/update response 2");
    assert!(
        s2_resp.get("result").is_some(),
        "settings/update(effort=low) 应成功: {s2_resp}"
    );

    cleanup_thread(&mut server, &thread_id, deadline);
}

#[test]
fn app_server_turns_list_full_and_search() {
    if codex_bin().is_none() {
        eprintln!("跳过：未设置 CODEX_BIN");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut server = Server::start(tmp.path());
    let deadline = Instant::now() + Duration::from_secs(120);
    let thread_id = start_thread(&mut server, tmp.path(), deadline);

    // 唯一搜索词（PID + 纳秒），避免命中历史残留会话
    let marker = format!(
        "SMOKE{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let prompt = format!("回复包含 {marker} 的一个词");

    let turn_id = server.request(
        "turn/start",
        json!({
            "threadId": thread_id,
            "input": [{"type": "text", "text": prompt, "text_elements": []}]
        }),
    );
    let turn_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(turn_id))
        .expect("turn/start response");
    assert!(turn_resp.get("result").is_some(), "turn/start 应成功: {turn_resp}");
    let completed = server
        .wait_for(deadline, |v| {
            v.get("method").and_then(|m| m.as_str()) == Some("turn/completed")
        })
        .expect("turn/completed");
    assert_eq!(
        completed["params"]["turn"]["status"].as_str().unwrap_or(""),
        "completed"
    );

    // turns/list：asc + full（协议 SortDirection 为 "asc"|"desc"），完整条目应包含用户消息
    let list_id = server.request(
        "thread/turns/list",
        json!({
            "threadId": thread_id,
            "sortDirection": "asc",
            "itemsView": "full",
            "limit": 50
        }),
    );
    let list_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(list_id))
        .expect("thread/turns/list response");
    assert!(list_resp.get("result").is_some(), "turns/list 应成功: {list_resp}");
    let full_text = serde_json::to_string(&list_resp["result"]).unwrap_or_default();
    assert!(
        full_text.contains(&marker),
        "turns/list(full) 应包含用户消息文本: {full_text}"
    );

    // thread/search：按唯一词命中新建会话（索引可能延迟，短重试窗口）
    let mut found = false;
    for _ in 0..30 {
        let search_id = server.request("thread/search", json!({ "searchTerm": marker }));
        let search_resp = server
            .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(search_id))
            .expect("thread/search response");
        assert!(search_resp.get("result").is_some(), "thread/search 应成功: {search_resp}");
        if search_resp["result"]["data"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        {
            found = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(found, "thread/search 应命中新建会话（marker={marker}）");

    cleanup_thread(&mut server, &thread_id, deadline);
}

#[test]
fn app_server_pin_unpin_section_move() {
    if codex_bin().is_none() {
        eprintln!("跳过：未设置 CODEX_BIN");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut server = Server::start(tmp.path());
    let deadline = Instant::now() + Duration::from_secs(90);

    let init_id = server.request(
        "initialize",
        json!({
            "clientInfo": { "name": "codex-ui-test", "title": "Codex UI Test", "version": "0.1.0" },
            "capabilities": { "experimentalApi": true, "requestAttestation": false }
        }),
    );
    let init_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(init_id))
        .expect("initialize response");
    assert!(init_resp.get("result").is_some(), "initialize 应成功: {init_resp}");
    server.notify("initialized");

    // 固定协议（0.149+）：threadSection/list 定位 Pinned 分区 → thread/section/move
    let sec_id = server.request("threadSection/list", json!({ "limit": 50 }));
    let sec_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(sec_id))
        .expect("threadSection/list response");
    let sec_result = sec_resp
        .get("result")
        .expect("threadSection/list 应成功: {sec_resp}");
    let data = sec_result["data"].as_array().expect("sections array");
    let pinned_id = data
        .iter()
        .find(|s| s["name"] == "Pinned")
        .or_else(|| data.iter().find(|s| s["id"] == PINNED_SECTION_ID))
        .and_then(|s| s["id"].as_str())
        .unwrap_or(PINNED_SECTION_ID)
        .to_string();

    // 需要一个已持久化（出现在列表里）的线程：跑完一个回合
    let start_id = server.request(
        "thread/start",
        json!({
            "cwd": tmp.path().to_string_lossy(),
            "approvalPolicy": "untrusted",
            "sandbox": "read-only"
        }),
    );
    let start_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(start_id))
        .expect("thread/start response");
    let thread_id = start_resp["result"]["thread"]["id"]
        .as_str()
        .expect("thread id")
        .to_string();

    let turn_id = server.request(
        "turn/start",
        json!({
            "threadId": thread_id,
            "input": [{"type": "text", "text": "只回复 OK 两个字母", "text_elements": []}]
        }),
    );
    let turn_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(turn_id))
        .expect("turn/start response");
    assert!(turn_resp.get("result").is_some(), "turn/start 应成功: {turn_resp}");
    let completed = server
        .wait_for(deadline, |v| {
            v.get("method").and_then(|m| m.as_str()) == Some("turn/completed")
        })
        .expect("turn/completed");
    assert_eq!(completed["params"]["turn"]["status"].as_str().unwrap_or(""), "completed");

    // thread/section/move 置顶
    let pin_id = server.request(
        "thread/section/move",
        json!({ "threadId": thread_id, "sectionId": pinned_id }),
    );
    let pin_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(pin_id))
        .expect("pin response");
    assert!(pin_resp.get("result").is_some(), "置顶应成功: {pin_resp}");

    // 读回：验证 section 指向 Pinned 分区
    let read_id = server.request(
        "thread/read",
        json!({ "threadId": thread_id, "includeTurns": false }),
    );
    let read_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(read_id))
        .expect("thread/read response");
    let thread = &read_resp["result"]["thread"];
    let section = thread["section"]
        .as_object()
        .expect("置顶后 thread/read 应返回 section");
    assert_eq!(
        section["id"].as_str().unwrap_or(""),
        pinned_id,
        "section id 应等于 Pinned: {read_resp}"
    );

    // thread/section/move 取消置顶
    let un_id = server.request(
        "thread/section/move",
        json!({ "threadId": thread_id, "sectionId": null }),
    );
    let un_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(un_id))
        .expect("unpin response");
    assert!(un_resp.get("result").is_some(), "取消置顶应成功: {un_resp}");

    let read2_id = server.request(
        "thread/read",
        json!({ "threadId": thread_id, "includeTurns": false }),
    );
    let read2_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(read2_id))
        .expect("thread/read response 2");
    let thread2 = &read2_resp["result"]["thread"];
    assert!(
        thread2["section"].is_null(),
        "取消置顶后 section 应为 null: {read2_resp}"
    );

    let delete_id = server.request("thread/delete", json!({ "threadId": thread_id }));
    let _ = server.wait_for(deadline, |v| {
        v.get("id").and_then(|i| i.as_u64()) == Some(delete_id)
    });

    server.child.kill().ok();
    let _ = server.child.wait();
}

#[test]
fn settings_roundtrip() {
    use codex_ui_lib::codex::settings;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut s = settings::load(tmp.path());
    assert_eq!(s.followup_mode, "adjust");
    assert!(s.enter_to_send);
    assert!(s.interaction_notify_enabled);
    s.followup_mode = "queue".into();
    s.enter_to_send = false;
    s.interaction_notify_enabled = false;
    settings::save(tmp.path(), &s).expect("save");
    let loaded = settings::load(tmp.path());
    assert_eq!(loaded.followup_mode, "queue");
    assert!(!loaded.enter_to_send);
    assert!(!loaded.interaction_notify_enabled);
}
