//! 知识库 CLI 端到端用例：create → 建库 → 检索 → 改文件增量 → 删文件清理 → 跨目录共享。
//!
//! 需要真实向量模型与 ONNX Runtime，所以默认跳过：仅当 **CLI 同目录**存在
//! `model/bge-small-zh-v1.5/model.onnx` 时才运行（用
//! `pwsh -File scripts/build-knowledge-model.ps1 -AlsoDev` 生成）。
//! 数据目录与 ONNX Runtime 都通过临时环境变量隔离/指定（`APPDATA`、`CODEXUI_ORT_DYLIB`）。

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn cli_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_codexui-kb"))
}

/// CLI 同目录下模型是否就绪（不就绪则跳过用例）
fn model_ready() -> bool {
    let exe = cli_bin();
    let Some(dir) = exe.parent() else {
        return false;
    };
    dir.join("model")
        .join("bge-small-zh-v1.5")
        .join("model.onnx")
        .is_file()
}

/// 运行 CLI：`APPDATA` 指到临时数据根，stdout 逐行解析 NDJSON
fn run_cli(appdata: &Path, args: &[String]) -> (i32, Vec<Value>) {
    let output = Command::new(cli_bin())
        .args(args)
        .env("APPDATA", appdata)
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

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn create_index_search_incremental_and_shared_across_dirs() {
    if !model_ready() {
        eprintln!(
            "跳过：CLI 同目录没有模型（先运行 pwsh -File scripts/build-knowledge-model.ps1 -AlsoDev）"
        );
        return;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let appdata = tmp.path().join("AppData");
    std::fs::create_dir_all(&appdata).unwrap();
    let ws_a = tmp.path().join("ws-a");
    let ws_b = tmp.path().join("ws-b");
    std::fs::create_dir_all(&ws_a).unwrap();
    std::fs::create_dir_all(&ws_b).unwrap();
    let doc = ws_a.join("手册.md");
    std::fs::write(
        &doc,
        "# 售后手册\n\n## 3.2 无法开机\n\n错误码 E-1043 表示电源适配器异常，请更换适配器后重试。",
    )
    .unwrap();
    let ws_a_s = ws_a.to_string_lossy().to_string();

    // create：登记库名与来源目录
    let (code, lines) = run_cli(&appdata, &args(&["create", "售后手册", &ws_a_s]));
    assert_eq!(code, 0, "create 应成功：{:?}", error_of(&lines));
    assert_eq!(result_of(&lines)["created"], true);

    // 幂等：同来源再 create → created=false
    let (code, lines) = run_cli(&appdata, &args(&["create", "售后手册", &ws_a_s]));
    assert_eq!(code, 0);
    assert_eq!(result_of(&lines)["created"], false);

    // index：首次建库应新增 1 篇并产生切块
    let (code, lines) = run_cli(&appdata, &args(&["index", "售后手册"]));
    assert_eq!(code, 0, "建库应成功：{:?}", error_of(&lines));
    let summary = &result_of(&lines)["summary"];
    assert_eq!(summary["added"], 1, "首次建库应新增 1 篇：{summary}");
    assert!(summary["chunks"].as_i64().unwrap() >= 1);
    assert_eq!(summary["source"], ws_a_s);
    assert!(lines
        .iter()
        .any(|v| v.get("type").and_then(Value::as_str) == Some("progress")));

    // 落盘位置：库文件在 <APPDATA>\com.codexui.app\kbs\，且不再创建 knowledge\ 目录
    let kb_dir = appdata.join("com.codexui.app").join("kbs");
    let files: Vec<String> = std::fs::read_dir(&kb_dir)
        .expect("kbs 目录应存在")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        files.iter().any(|f| f.starts_with("售后手册-") && f.ends_with(".sqlite")),
        "库文件应落在 kbs 下：{files:?}"
    );
    assert!(
        !appdata.join("com.codexui.app").join("knowledge").exists(),
        "不应再创建 knowledge 子目录"
    );

    // search：命中含错误码的切块，并带回库名与来源
    let (code, lines) = run_cli(
        &appdata,
        &args(&["search", "售后手册", "--query", "错误码 E-1043 电源适配器"]),
    );
    assert_eq!(code, 0, "检索应成功：{:?}", error_of(&lines));
    let hits = result_of(&lines)["hits"].as_array().unwrap().clone();
    assert!(!hits.is_empty(), "应至少命中 1 条：{lines:?}");
    assert!(hits[0]["text"].as_str().unwrap().contains("E-1043"));
    assert!(hits[0]["doc_path"].as_str().unwrap().ends_with("手册.md"));
    assert_eq!(result_of(&lines)["kb"], "售后手册");

    // 内容未变：再次 index 应全部跳过（幂等）
    let (code, lines) = run_cli(&appdata, &args(&["index", "售后手册"]));
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
    let (code, lines) = run_cli(&appdata, &args(&["index", "售后手册"]));
    assert_eq!(code, 0);
    let summary = &result_of(&lines)["summary"];
    assert_eq!(summary["updated"], 1, "增量应只更新变更文件：{summary}");
    assert_eq!(summary["added"], 0);

    // 同名不同来源：create 必须报错且不改写
    let (code, lines) = run_cli(
        &appdata,
        &args(&["create", "售后手册", &ws_b.to_string_lossy()]),
    );
    assert_ne!(code, 0);
    assert!(error_of(&lines).unwrap().contains("来源"));

    // 跨目录共享：另一个工作目录用同一个库名检索，命中同一批内容
    let (code, lines) = run_cli(
        &appdata,
        &args(&["search", "售后手册", "--query", "错误码 E-2001 电池温度"]),
    );
    assert_eq!(code, 0);
    let hits = result_of(&lines)["hits"].as_array().unwrap();
    assert!(!hits.is_empty(), "跨目录检索应命中：{lines:?}");

    // 另一个库与它互不干扰
    std::fs::write(
        ws_b.join("记录.md"),
        "# 工单\n\n记录：更换风扇后温度恢复正常。",
    )
    .unwrap();
    let (code, _) = run_cli(
        &appdata,
        &args(&["create", "工单", &ws_b.to_string_lossy()]),
    );
    assert_eq!(code, 0);
    let (code, lines) = run_cli(&appdata, &args(&["index", "工单"]));
    assert_eq!(code, 0, "{:?}", error_of(&lines));
    let (code, lines) = run_cli(&appdata, &args(&["list"]));
    assert_eq!(code, 0);
    assert_eq!(result_of(&lines)["kbs"].as_array().unwrap().len(), 2);

    // 删文件：只清理本库来源内的失效文档
    std::fs::remove_file(&doc).unwrap();
    let (code, lines) = run_cli(&appdata, &args(&["index", "售后手册"]));
    assert_eq!(code, 0, "{:?}", error_of(&lines));
    let summary = &result_of(&lines)["summary"];
    assert_eq!(summary["removed"], 1, "应移除失效文档：{summary}");
    assert_eq!(summary["docs"], 0);
    let (code, lines) = run_cli(&appdata, &args(&["list"]));
    assert_eq!(code, 0);
    let rows = result_of(&lines)["kbs"].as_array().unwrap();
    let gong_dan = rows.iter().find(|r| r["kb"] == "工单").unwrap();
    assert_eq!(gong_dan["docs"], 1, "另一个库不受影响：{gong_dan}");

    // delete 按库名
    let (code, lines) = run_cli(&appdata, &args(&["delete", "售后手册"]));
    assert_eq!(code, 0, "{:?}", error_of(&lines));
    let (code, lines) = run_cli(&appdata, &args(&["list"]));
    assert_eq!(code, 0);
    assert_eq!(result_of(&lines)["kbs"].as_array().unwrap().len(), 1);
}
