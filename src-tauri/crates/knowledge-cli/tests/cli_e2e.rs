//! 知识库 CLI 端到端用例：建库 → 检索 → 改文件增量 → 删文件清理。
//!
//! 需要真实的向量模型与 ONNX Runtime，所以默认跳过；设置以下环境变量后启用：
//! - `CODEXUI_KB_MODEL_DIR`：模型根目录（内含 `bge-small-zh-v1.5/`），仅用于「有真模型才跑」的跳过判定；
//!   平时它就是 `setup/bin/model`（或 `src-tauri/target/debug/model`）
//! - `CODEXUI_ORT_DYLIB`：onnxruntime.dll 的绝对路径（或把它放到 codexui-kb.exe 同目录）

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn model_root() -> Option<PathBuf> {
    let raw = std::env::var_os("CODEXUI_KB_MODEL_DIR")?;
    let dir = PathBuf::from(raw);
    if dir.join("bge-small-zh-v1.5").join("model.onnx").is_file() {
        Some(dir)
    } else {
        None
    }
}

fn run_cli(args: &[String]) -> (i32, Vec<Value>) {
    let bin = env!("CARGO_BIN_EXE_codexui-kb");
    let output = Command::new(bin)
        .args(args)
        .output()
        .expect("应能启动 codexui-kb");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let lines = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect("stdout 每行都应是 JSON"))
        .collect();
    (output.status.code().unwrap_or(-1), lines)
}

fn result_of(lines: &[Value]) -> &Value {
    lines
        .iter()
        .find(|v| v.get("type").and_then(Value::as_str) == Some("result"))
        .expect("应有 result 行")
}

fn error_of(lines: &[Value]) -> Option<String> {
    lines
        .iter()
        .find(|v| v.get("type").and_then(Value::as_str) == Some("error"))
        .and_then(|v| v.get("message"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn arg(name: &str, value: impl AsRef<str>) -> Vec<String> {
    vec![name.to_string(), value.as_ref().to_string()]
}

/// 模型实例目录（`<根>/bge-small-zh-v1.5`）：CLI 的 `--model-dir` 指向它
fn model_instance(model_root: &Path) -> PathBuf {
    model_root.join("bge-small-zh-v1.5")
}

fn index_args(data_dir: &Path, cwd: &Path, model_dir: &Path) -> Vec<String> {
    let mut args = vec!["index".to_string()];
    args.extend(arg("--data-dir", data_dir.to_string_lossy()));
    args.extend(arg("--cwd", cwd.to_string_lossy()));
    args.extend(arg("--model-dir", model_dir.to_string_lossy()));
    args
}

#[test]
fn incremental_index_and_search_roundtrip() {
    let Some(model_root) = model_root() else {
        eprintln!("跳过：未设置 CODEXUI_KB_MODEL_DIR（或模型文件不完整）");
        return;
    };
    let model_dir = model_instance(&model_root);
    let tmp = tempfile::TempDir::new().unwrap();
    let data_dir = tmp.path().join("knowledge");
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let doc = ws.join("手册.md");
    std::fs::write(
        &doc,
        "# 售后手册\n\n## 3.2 无法开机\n\n错误码 E-1043 表示电源适配器异常，请更换适配器后重试。",
    )
    .unwrap();

    // 首次建库：应新增 1 篇文档且产生切块
    let (code, lines) = run_cli(&index_args(&data_dir, &ws, &model_dir));
    assert_eq!(code, 0, "建库应成功：{:?}", error_of(&lines));
    let summary = &result_of(&lines)["summary"];
    assert_eq!(summary["added"], 1, "首次建库应新增 1 篇：{summary}");
    assert!(summary["docs"].as_i64().unwrap() >= 1);
    assert!(summary["chunks"].as_i64().unwrap() >= 1);
    assert!(!lines.iter().any(|v| v.get("type").unwrap() == "error"));

    // 检索：命中含错误码的切块，并带回出处
    let mut search_args = vec!["search".to_string()];
    search_args.extend(arg("--data-dir", data_dir.to_string_lossy()));
    search_args.extend(arg("--cwd", ws.to_string_lossy()));
    search_args.extend(arg("--model-dir", model_dir.to_string_lossy()));
    search_args.extend(arg("--query", "电源适配器异常错误码"));
    let (code, lines) = run_cli(&search_args);
    assert_eq!(code, 0, "检索应成功：{:?}", error_of(&lines));
    let hits = result_of(&lines)["hits"].as_array().unwrap().clone();
    assert!(!hits.is_empty(), "应至少命中 1 条：{lines:?}");
    assert!(hits[0]["text"].as_str().unwrap().contains("E-1043"));
    assert!(hits[0]["doc_path"]
        .as_str()
        .unwrap()
        .ends_with("手册.md"));

    // 内容未变：再次建库应全部跳过（幂等）
    let (code, lines) = run_cli(&index_args(&data_dir, &ws, &model_dir));
    assert_eq!(code, 0);
    let summary = &result_of(&lines)["summary"];
    assert_eq!(summary["added"], 0);
    assert_eq!(summary["updated"], 0);
    assert_eq!(summary["skipped"], 1);

    // 改文件：增量应只更新 1 篇
    std::fs::write(
        &doc,
        "# 售后手册\n\n## 3.2 无法开机\n\n错误码 E-1043 表示电源适配器异常；新增：E-2001 表示电池温度过高。",
    )
    .unwrap();
    let (code, lines) = run_cli(&index_args(&data_dir, &ws, &model_dir));
    assert_eq!(code, 0);
    let summary = &result_of(&lines)["summary"];
    assert_eq!(summary["updated"], 1, "增量应只更新变更文件：{summary}");
    assert_eq!(summary["added"], 0);

    // 删文件：应清理失效文档
    std::fs::remove_file(&doc).unwrap();
    let (code, lines) = run_cli(&index_args(&data_dir, &ws, &model_dir));
    assert_eq!(code, 0);
    let summary = &result_of(&lines)["summary"];
    assert_eq!(summary["removed"], 1, "应移除失效文档：{summary}");
    assert_eq!(summary["docs"], 0);
}
