//! 解析 codex 会话 .jsonl 文件，还原完整历史条目。
//!
//! 背景：当前 codex app-server 的 thread/read 与 thread/turns/list 对历史会话
//! 只返回“重建的简化视图”（仅消息与文件变更），命令执行、推理、工具调用等条目
//! 在接口层被丢弃。完整数据仍保存在 `~/.codex/sessions/**/rollout-*.jsonl`，
//! codex CLI 的历史回放就是直接读取该文件。这里做同样的解析，
//! 把 response_item / event_msg 记录还原成前端可渲染的 ThreadItem。

use serde_json::{Value, json};
use std::collections::HashMap;

/// 把会话文件全文解析为 ThreadItem 数组（顺序与文件一致）。
pub fn parse_session_items(text: &str, fallback_cwd: &str) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut cwd = fallback_cwd.to_string();
    // call_id -> (名称, 参数, 条目 id, 起始时间)
    let mut pending: Vec<(String, String, Value, String, i64)> = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ts_ms = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .map(iso_to_ms)
            .unwrap_or(0);
        let Some(payload) = v.get("payload") else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("session_meta") => {
                if let Some(c) = payload.get("cwd").and_then(|x| x.as_str()) {
                    cwd = c.to_string();
                }
            }
            Some("event_msg") => {
                if payload.get("type").and_then(|t| t.as_str()) == Some("user_message") {
                    let text = payload
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string();
                    let images: Vec<String> = payload
                        .get("local_images")
                        .and_then(|a| a.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let mut content: Vec<Value> = vec![json!({
                        "type": "text",
                        "text": text,
                        "text_elements": []
                    })];
                    for img in images {
                        content.push(json!({ "type": "localImage", "path": img }));
                    }
                    let id = payload
                        .get("client_id")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    out.push(json!({ "id": id, "type": "userMessage", "content": content }));
                }
            }
            Some("response_item") => {
                let it = payload;
                let itype = it.get("type").and_then(|t| t.as_str()).unwrap_or("");
                let id = it
                    .get("id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                match itype {
                    "message" => {
                        let role = it.get("role").and_then(|r| r.as_str()).unwrap_or("");
                        if role == "assistant" {
                            let text = first_content_text(it);
                            let phase = it
                                .get("phase")
                                .and_then(|p| p.as_str())
                                .unwrap_or("final_answer")
                                .to_string();
                            out.push(json!({
                                "id": id,
                                "type": "agentMessage",
                                "text": text,
                                "phase": phase
                            }));
                        }
                        // role=user 用户消息由 event_msg 提供；role=developer 为系统提示，不展示
                    }
                    "reasoning" => {
                        let content: Vec<String> = it
                            .get("content")
                            .and_then(|c| c.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| {
                                        x.get("text")
                                            .and_then(|t| t.as_str())
                                            .map(|s| s.to_string())
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        out.push(json!({
                            "id": id,
                            "type": "reasoning",
                            "summary": [],
                            "content": content,
                            "startedAtMs": ts_ms,
                            "completedAtMs": ts_ms
                        }));
                    }
                    "function_call" => {
                        let name = it
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        let call_id = it
                            .get("call_id")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .to_string();
                        let args = it
                            .get("arguments")
                            .and_then(|a| a.as_str())
                            .and_then(|s| serde_json::from_str::<Value>(s).ok())
                            .unwrap_or(Value::Null);
                        pending.push((call_id, name, args, id, ts_ms));
                    }
                    "function_call_output" => {
                        let call_id = it
                            .get("call_id")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .to_string();
                        let output = it
                            .get("output")
                            .and_then(|o| o.as_str())
                            .unwrap_or("")
                            .to_string();
                        if let Some(idx) = pending.iter().position(|(cid, _, _, _, _)| *cid == call_id)
                        {
                            let (_, name, args, fid, ts0) = pending.remove(idx);
                            if name == "shell_command" || name == "exec_command" {
                                let command = args
                                    .get("command")
                                    .and_then(|c| c.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                out.push(json!({
                                    "id": fid,
                                    "type": "commandExecution",
                                    "command": command,
                                    "cwd": cwd,
                                    "status": "completed",
                                    "aggregatedOutput": output,
                                    "exitCode": parse_exit_code(&output),
                                    "startedAtMs": ts0,
                                    "completedAtMs": ts_ms,
                                    "durationMs": parse_wall_time_ms(&output)
                                }));
                            } else {
                                out.push(json!({
                                    "id": fid,
                                    "type": "dynamicToolCall",
                                    "tool": name,
                                    "arguments": args,
                                    "status": "completed",
                                    "contentItems": [],
                                    "success": true,
                                    "startedAtMs": ts0,
                                    "completedAtMs": ts_ms
                                }));
                            }
                        }
                    }
                    "custom_tool_call" => {
                        let name = it
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        let status = it
                            .get("status")
                            .and_then(|s| s.as_str())
                            .unwrap_or("completed")
                            .to_string();
                        let call_id = it
                            .get("call_id")
                            .and_then(|c| c.as_str())
                            .unwrap_or(&id)
                            .to_string();
                        if name == "apply_patch" {
                            let input = it
                                .get("input")
                                .and_then(|i| i.as_str())
                                .unwrap_or("")
                                .to_string();
                            out.push(json!({
                                "id": call_id,
                                "type": "fileChange",
                                "changes": parse_apply_patch(&input),
                                "status": "completed"
                            }));
                        } else {
                            let input = it.get("input").cloned().unwrap_or(Value::Null);
                            out.push(json!({
                                "id": call_id,
                                "type": "dynamicToolCall",
                                "tool": name,
                                "arguments": input,
                                "status": status
                            }));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

fn first_content_text(it: &Value) -> String {
    it.get("content")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c0| c0.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

/// 从命令输出中解析 "Exit code: N"（N 允许负数）。
fn parse_exit_code(output: &str) -> Option<i64> {
    let lower = output.to_ascii_lowercase();
    let marker = "exit code:";
    let idx = lower.find(marker)?;
    let rest = &lower[idx + marker.len()..];
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    digits.parse::<i64>().ok()
}

/// 从命令输出中解析 "Wall time: X.Y seconds" 为毫秒。
fn parse_wall_time_ms(output: &str) -> Option<i64> {
    let lower = output.to_ascii_lowercase();
    let marker = "wall time:";
    let idx = lower.find(marker)?;
    let rest = &lower[idx + marker.len()..];
    let num: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse::<f64>().ok().map(|s| (s * 1000.0) as i64)
}

/// 解析 apply_patch 输入，还原为 { path, kind, diff } 变更列表。
fn parse_apply_patch(input: &str) -> Vec<Value> {
    let mut changes: Vec<Value> = Vec::new();
    let mut cur_path: Option<String> = None;
    let mut cur_kind = "update";
    let mut cur_lines: Vec<&str> = Vec::new();
    let flush = |path: &Option<String>, kind: &str, lines: &mut Vec<&str>, changes: &mut Vec<Value>| {
        if let Some(p) = path {
            changes.push(json!({
                "path": p,
                "kind": { "type": kind, "move_path": null },
                "diff": lines.join("\n")
            }));
        }
        lines.clear();
    };
    for line in input.lines() {
        let t = line.trim_start();
        if t.starts_with("*** ") {
            // 新文件头：*** Update File: path / *** Add File: path / *** Delete File: path
            let header = t.trim_start_matches("*** ").trim();
            let (kind, path) = if let Some(p) = header.strip_prefix("Update File: ") {
                ("update", Some(p.trim().to_string()))
            } else if let Some(p) = header.strip_prefix("Add File: ") {
                ("add", Some(p.trim().to_string()))
            } else if let Some(p) = header.strip_prefix("Delete File: ") {
                ("delete", Some(p.trim().to_string()))
            } else if let Some(p) = header.strip_prefix("Move to: ") {
                ("move", Some(p.trim().to_string()))
            } else {
                ("update", None)
            };
            flush(&cur_path, cur_kind, &mut cur_lines, &mut changes);
            cur_path = path;
            cur_kind = kind;
        } else {
            cur_lines.push(line);
        }
    }
    flush(&cur_path, cur_kind, &mut cur_lines, &mut changes);
    changes
}

/// "2026-08-08T18:08:40.285Z" -> 毫秒（UTC）。
fn iso_to_ms(s: &str) -> i64 {
    let s = s.trim();
    if s.len() < 19 {
        return 0;
    }
    let y: i64 = s[0..4].parse().unwrap_or(0);
    let mo: i64 = s[5..7].parse().unwrap_or(0);
    let d: i64 = s[8..10].parse().unwrap_or(0);
    let h: i64 = s[11..13].parse().unwrap_or(0);
    let mi: i64 = s[14..16].parse().unwrap_or(0);
    let sec: i64 = s[17..19].parse().unwrap_or(0);
    let frac: i64 = if s.len() > 20 {
        s[20..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<i64>()
            .unwrap_or(0)
    } else {
        0
    };
    let days = days_from_civil(y, mo, d);
    (days * 86400 + h * 3600 + mi * 60 + sec) * 1000 + frac
}

/// 简易公历日序（Hinnant 算法），1970-01-01 = 0。
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        r#"{"timestamp":"2026-08-04T11:55:59.000Z","type":"session_meta","payload":{"cwd":"D:\\proj","history_mode":"legacy"}}
{"timestamp":"2026-08-04T11:56:00.100Z","type":"event_msg","payload":{"type":"user_message","client_id":"user-1","message":"检查项目","local_images":["C:\\a.png"]}}
{"timestamp":"2026-08-04T11:56:01.000Z","type":"response_item","payload":{"type":"message","id":"msg-1","role":"assistant","content":[{"type":"output_text","text":"我先看一下"}],"phase":"commentary"}}
{"timestamp":"2026-08-04T11:56:02.000Z","type":"response_item","payload":{"type":"reasoning","id":"r-1","content":[{"type":"reasoning_text","text":"思考中"}]}}
{"timestamp":"2026-08-04T11:56:03.000Z","type":"response_item","payload":{"type":"function_call","id":"f-1","name":"shell_command","arguments":"{\"command\":\"Get-ChildItem\"}","call_id":"call-1"}}
{"timestamp":"2026-08-04T11:56:04.500Z","type":"response_item","payload":{"type":"function_call_output","id":"o-1","call_id":"call-1","output":"Exit code: 0\nWall time: 2 seconds\nOutput:\nok"}}
{"timestamp":"2026-08-04T11:56:05.000Z","type":"response_item","payload":{"type":"custom_tool_call","id":"c-1","status":"completed","call_id":"call-2","name":"apply_patch","input":"*** Update File: D:\\proj\\a.cs\n@@\n-old\n+new\n"}}
{"timestamp":"2026-08-04T11:56:06.000Z","type":"response_item","payload":{"type":"message","id":"msg-2","role":"assistant","content":[{"type":"output_text","text":"完成"}],"phase":"final_answer"}}
"#
        .to_string()
    }

    #[test]
    fn parses_full_items() {
        let items = parse_session_items(&fixture(), "D:\\fallback");
        let types: Vec<&str> = items.iter().filter_map(|i| i["type"].as_str()).collect();
        assert_eq!(
            types,
            vec!["userMessage", "agentMessage", "reasoning", "commandExecution", "fileChange", "agentMessage"]
        );
        // 用户消息带图片附件
        let user = &items[0];
        assert_eq!(user["content"][1]["type"], "localImage");
        assert_eq!(user["content"][1]["path"], "C:\\a.png");
        // 命令条目
        let cmd = &items[3];
        assert_eq!(cmd["command"], "Get-ChildItem");
        assert_eq!(cmd["cwd"], "D:\\proj");
        assert_eq!(cmd["exitCode"], 0);
        assert_eq!(cmd["durationMs"], 2000);
        assert_eq!(cmd["aggregatedOutput"], "Exit code: 0\nWall time: 2 seconds\nOutput:\nok");
        // 文件变更
        let fc = &items[4];
        assert_eq!(fc["changes"][0]["path"], "D:\\proj\\a.cs");
        assert!(fc["changes"][0]["diff"].as_str().unwrap().contains("-old\n+new"));
        // 忽略 developer 消息 / 独立输出
        assert_eq!(items.len(), 6);
    }

    #[test]
    fn exit_code_negative_and_wall_time() {
        assert_eq!(parse_exit_code("Exit code: -1"), Some(-1));
        assert_eq!(parse_wall_time_ms("Wall time: 1.5 seconds"), Some(1500));
    }

    #[test]
    fn iso_parsing() {
        assert_eq!(iso_to_ms("1970-01-01T00:00:00.000Z"), 0);
        assert_eq!(iso_to_ms("1970-01-01T00:00:01.500Z"), 1500);
    }
}
