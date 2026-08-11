//! 集成冒烟测试：直接驱动真实 `codex app-server --stdio`，验证本项目依赖的协议流程。
//! 设置环境变量 CODEX_BIN 指向 codex 可执行文件时才会运行，否则跳过。

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

fn codex_bin() -> Option<String> {
    std::env::var("CODEX_BIN").ok()
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

#[test]
fn app_server_pin_unpin_via_section() {
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

    // 新版协议通过内置 “Pinned” 分区实现置顶；旧版不支持时跳过
    let sec_id = server.request("threadSection/list", json!({ "limit": 50 }));
    let sec_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(sec_id))
        .expect("threadSection/list response");
    let Some(result) = sec_resp.get("result") else {
        eprintln!("跳过：当前 codex 不支持 threadSection/list");
        server.child.kill().ok();
        let _ = server.child.wait();
        return;
    };
    let data = result["data"].as_array().expect("sections array");
    let pinned = data
        .iter()
        .find(|s| s["name"] == "Pinned")
        .or_else(|| data.first());
    let Some(pinned) = pinned else {
        eprintln!("跳过：未发现 Pinned 分区");
        server.child.kill().ok();
        let _ = server.child.wait();
        return;
    };
    let pinned_id = pinned["id"].as_str().expect("section id").to_string();

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

    // 置顶：metadata/update 携带 Pinned 分区 id
    let pin_id = server.request(
        "thread/metadata/update",
        json!({ "threadId": thread_id, "sectionId": pinned_id }),
    );
    let pin_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(pin_id))
        .expect("pin response");
    assert!(pin_resp.get("result").is_some(), "置顶应成功: {pin_resp}");

    let read_id = server.request(
        "thread/read",
        json!({ "threadId": thread_id, "includeTurns": false }),
    );
    let read_resp = server
        .wait_for(deadline, |v| v.get("id").and_then(|i| i.as_u64()) == Some(read_id))
        .expect("thread/read response");
    let section = read_resp["result"]["thread"]["section"]
        .as_object()
        .expect("置顶后 thread/read 应返回 section");
    assert_eq!(section["id"].as_str().unwrap_or(""), pinned_id, "section id 应等于 Pinned: {read_resp}");

    // 取消置顶：sectionId 置 null
    let un_id = server.request(
        "thread/metadata/update",
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
    assert!(
        read2_resp["result"]["thread"]["section"].is_null(),
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
    assert!(s.sound_enabled);
    s.followup_mode = "queue".into();
    s.enter_to_send = false;
    s.sound_enabled = false;
    settings::save(tmp.path(), &s).expect("save");
    let loaded = settings::load(tmp.path());
    assert_eq!(loaded.followup_mode, "queue");
    assert!(!loaded.enter_to_send);
    assert!(!loaded.sound_enabled);
}
