//! 命令行协议层：参数解析 + 面向 codex-ui 的 NDJSON 输出。
//!
//! stdout 协议（每行一个 JSON 对象，UTF-8）：
//! - 若干 `{"type":"progress","cwd":…,"phase":"scan|index|done","processed":n,"total":n,"current":"…"}`
//! - 最后恰好一条 `{"type":"result",…}`（`index` 带 `summary`、`search` 带 `hits`、
//!   `list` 带 `kbs`、`delete` 带 `ok`、`version` 带 `version` 与 `codexUiApi`）
//! - 或 `{"type":"error","code":n,"message":"…"}`（中文，可直接展示给用户）
//!
//! stderr 只放人可读日志，codex-ui 只在失败时截取尾部用于诊断。
//!
//! 退出码：0 成功、1 内部错误、2 参数错误、3 模型/ONNX Runtime 未就绪、
//! 4 知识库被占用、5 保留给"已取消"（当前由父进程杀进程实现，不主动返回）。

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use serde_json::json;

use crate::{embed, index, paths, search, store};

/// 协议版本：codex-ui 用它校验 CLI 与自身是否匹配
pub const API_VERSION: u32 = 1;

pub const EXIT_OK: i32 = 0;
pub const EXIT_INTERNAL: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_MODEL: i32 = 3;
pub const EXIT_BUSY: i32 = 4;
pub const EXIT_CANCELLED: i32 = 5;

const USAGE: &str = "\
codexui-kb —— codex-ui 知识库 CLI

用法：
  codexui-kb index  --data-dir <知识库根目录> --cwd <工作目录> [--roots <路径> ...] [--full] [--model-dir <模型目录>]
  codexui-kb search --data-dir <知识库根目录> --cwd <工作目录> --query <检索词> [--top-k <n>] [--model-dir <模型目录>]
  codexui-kb list   --data-dir <知识库根目录>
  codexui-kb delete --data-dir <知识库根目录> --file <库文件名>
  codexui-kb version

--model-dir 缺省为「codexui-kb.exe 同目录的 model/bge-small-zh-v1.5」；
--data-dir 只承载各工作目录的 kbs/*.sqlite 索引。

输出：stdout 为 NDJSON（progress / result / error），stderr 为人可读日志。
退出码：0 成功、1 内部错误、2 参数错误、3 模型未就绪、4 库被占用、5 已取消。";

/// CLI 错误：携带协议错误码与中文文案
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub code: i32,
    pub message: String,
}

impl CliError {
    fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn usage(message: impl Into<String>) -> Self {
        Self::new(EXIT_USAGE, message)
    }
}

/// 进程入口（真实 stdout）
pub fn run(args: Vec<String>) -> i32 {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    run_with(&args, &mut out)
}

/// 可注入 writer 的入口（测试用）
pub fn run_with<W: Write>(args: &[String], out: &mut W) -> i32 {
    match dispatch(args, out) {
        Ok(code) => code,
        Err(err) => {
            let _ = write_line(
                out,
                &json!({ "type": "error", "code": err.code, "message": err.message }),
            );
            err.code
        }
    }
}

fn write_line<W: Write>(out: &mut W, value: &serde_json::Value) -> Result<(), CliError> {
    let line = serde_json::to_string(value)
        .map_err(|e| CliError::new(EXIT_INTERNAL, format!("序列化输出失败：{e}")))?;
    out.write_all(line.as_bytes())
        .and_then(|_| out.write_all(b"\n"))
        .and_then(|_| out.flush())
        .map_err(|e| CliError::new(EXIT_INTERNAL, format!("写出结果失败：{e}")))
}

fn dispatch<W: Write>(args: &[String], out: &mut W) -> Result<i32, CliError> {
    let Some((command, rest)) = args.split_first() else {
        return Err(CliError::usage(format!("缺少子命令。\n\n{USAGE}")));
    };
    if matches!(command.as_str(), "-h" | "--help" | "help") {
        writeln!(out, "{USAGE}")
            .map_err(|e| CliError::new(EXIT_INTERNAL, format!("写出帮助失败：{e}")))?;
        return Ok(EXIT_OK);
    }
    match command.as_str() {
        "version" => cmd_version(out, rest),
        "index" => cmd_index(out, rest),
        "search" => cmd_search(out, rest),
        "list" => cmd_list(out, rest),
        "delete" => cmd_delete(out, rest),
        other => Err(CliError::usage(format!(
            "未知子命令：{other}\n\n{USAGE}"
        ))),
    }
}

fn cmd_version<W: Write>(out: &mut W, rest: &[String]) -> Result<i32, CliError> {
    if !rest.is_empty() {
        return Err(CliError::usage("version 不接受任何参数"));
    }
    write_line(
        out,
        &json!({
            "type": "result",
            "version": env!("CARGO_PKG_VERSION"),
            "codexUiApi": API_VERSION,
        }),
    )?;
    Ok(EXIT_OK)
}

fn cmd_index<W: Write>(out: &mut W, rest: &[String]) -> Result<i32, CliError> {
    let flags = Flags::parse(rest, &["--data-dir", "--cwd", "--roots", "--model-dir"], &["--full"])?;
    let data_dir = flags.required_path("--data-dir")?;
    let cwd = flags.required("--cwd")?;
    let model_dir = flags.model_dir()?;
    let roots_arg: Vec<String> = flags.get_all("--roots").to_vec();
    let full = flags.has("--full");

    ensure_model(&model_dir)?;
    let roots =
        index::resolve_roots(&cwd, &roots_arg).map_err(|e| CliError::usage(e))?;

    let cancel = AtomicBool::new(false);
    let mut on_progress = |p: index::IndexProgress| {
        // 父进程（codex-ui）退出会让 stdout 关闭：此时尽快收尾，避免留下孤儿进程
        if write_line(
            out,
            &json!({
                "type": "progress",
                "cwd": p.cwd,
                "phase": p.phase,
                "processed": p.processed,
                "total": p.total,
                "current": p.current,
            }),
        )
        .is_err()
        {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    };
    let summary = index::run_index(
        &data_dir,
        &model_dir,
        &cwd,
        &roots,
        full,
        &cancel,
        &mut on_progress,
    )
    .map_err(map_engine_error)?
    .finalized();
    write_line(out, &json!({ "type": "result", "summary": summary }))?;
    Ok(EXIT_OK)
}

fn cmd_search<W: Write>(out: &mut W, rest: &[String]) -> Result<i32, CliError> {
    let flags = Flags::parse(
        rest,
        &["--data-dir", "--cwd", "--query", "--top-k", "--model-dir"],
        &[],
    )?;
    let data_dir = flags.required_path("--data-dir")?;
    let cwd = flags.required("--cwd")?;
    let query = flags.required("--query")?;
    let model_dir = flags.model_dir()?;
    let top_k = match flags.get("--top-k") {
        Some(raw) => raw
            .parse::<usize>()
            .map_err(|_| CliError::usage(format!("--top-k 需要正整数，收到：{raw}")))?,
        None => search::DEFAULT_TOP_K,
    };
    if query.trim().is_empty() {
        return Err(CliError::usage("--query 不能为空"));
    }
    let Some(store) = store::KbStore::open_existing(&data_dir, &cwd).map_err(map_engine_error)?
    else {
        write_line(out, &json!({ "type": "result", "hits": [] }))?;
        return Ok(EXIT_OK);
    };
    ensure_model(&model_dir)?;
    let cache = search::VectorCache::default();
    let hits = search::search(&model_dir, &store, &cache, &query, top_k)
        .map_err(map_engine_error)?;
    write_line(out, &json!({ "type": "result", "hits": hits }))?;
    Ok(EXIT_OK)
}

fn cmd_list<W: Write>(out: &mut W, rest: &[String]) -> Result<i32, CliError> {
    let flags = Flags::parse(rest, &["--data-dir"], &[])?;
    let data_dir = flags.required_path("--data-dir")?;
    let kbs = store::list_all(&data_dir);
    write_line(out, &json!({ "type": "result", "kbs": kbs }))?;
    Ok(EXIT_OK)
}

fn cmd_delete<W: Write>(out: &mut W, rest: &[String]) -> Result<i32, CliError> {
    let flags = Flags::parse(rest, &["--data-dir", "--file"], &[])?;
    let data_dir = flags.required_path("--data-dir")?;
    let file = flags.required("--file")?;
    store::delete_file(&data_dir, &file).map_err(map_engine_error)?;
    write_line(out, &json!({ "type": "result", "ok": true }))?;
    Ok(EXIT_OK)
}

/// 模型与 ONNX Runtime 就绪校验：不通过时给出可操作中文提示（退出码 3）
fn ensure_model(model_dir: &std::path::Path) -> Result<(), CliError> {
    if embed::model_ready(model_dir) {
        return Ok(());
    }
    Err(CliError::new(EXIT_MODEL, embed::model_problem(model_dir)))
}

/// 引擎错误 → 协议错误：库被占用单独识别，其余按内部错误上报
fn map_engine_error(message: String) -> CliError {
    let lower = message.to_lowercase();
    if lower.contains("locked") || lower.contains("busy") {
        return CliError::new(
            EXIT_BUSY,
            format!("知识库正被其它进程占用（可能正在建库），请稍后重试：{message}"),
        );
    }
    CliError::new(EXIT_INTERNAL, message)
}

/// 极简 `--flag value` / `--flag` 解析器（未知参数即报错，避免静默忽略）
#[derive(Debug, Default)]
struct Flags {
    values: HashMap<String, Vec<String>>,
    bools: HashSet<String>,
}

impl Flags {
    fn parse(args: &[String], value_flags: &[&str], bool_flags: &[&str]) -> Result<Self, CliError> {
        let mut flags = Flags::default();
        let mut i = 0usize;
        while i < args.len() {
            let raw = args[i].as_str();
            if let Some(name) = raw.split('=').next().filter(|n| n.starts_with("--")) {
                let inline = raw.contains('=');
                if bool_flags.contains(&name) {
                    if inline {
                        return Err(CliError::usage(format!("{name} 是开关，不接受取值")));
                    }
                    flags.bools.insert(name.to_string());
                    i += 1;
                    continue;
                }
                if !value_flags.contains(&name) {
                    return Err(CliError::usage(format!("未知参数：{raw}")));
                }
                let value = if inline {
                    raw.split_once('=').map(|(_, v)| v.to_string()).unwrap_or_default()
                } else {
                    i += 1;
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| CliError::usage(format!("{name} 缺少取值")))? 
                };
                flags.values.entry(name.to_string()).or_default().push(value);
                i += 1;
                continue;
            }
            return Err(CliError::usage(format!("无法识别的参数：{raw}")));
        }
        Ok(flags)
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values
            .get(name)
            .and_then(|v| v.last())
            .map(|s| s.as_str())
    }

    fn get_all(&self, name: &str) -> &[String] {
        self.values.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    fn has(&self, name: &str) -> bool {
        self.bools.contains(name)
    }

    fn required(&self, name: &str) -> Result<String, CliError> {
        let value = self
            .get(name)
            .ok_or_else(|| CliError::usage(format!("缺少必填参数 {name}")))?;
        if value.trim().is_empty() {
            return Err(CliError::usage(format!("{name} 不能为空")));
        }
        Ok(value.to_string())
    }

    fn required_path(&self, name: &str) -> Result<PathBuf, CliError> {
        Ok(PathBuf::from(self.required(name)?))
    }

    /// 模型目录（内含 `model.onnx`/`tokenizer.json` 等）：
    /// 显式 `--model-dir` 优先，否则与 `codexui-kb.exe` 同目录的 `model/<MODEL_ID>`
    fn model_dir(&self) -> Result<PathBuf, CliError> {
        match self.get("--model-dir") {
            Some(raw) => Ok(PathBuf::from(raw)),
            None => paths::model_dir().map_err(|e| CliError::new(EXIT_MODEL, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn lines(args: &[&str]) -> (i32, Vec<serde_json::Value>) {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let mut buf: Vec<u8> = Vec::new();
        let code = run_with(&owned, &mut buf);
        let text = String::from_utf8(buf).unwrap();
        let parsed = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("每行都应是合法 JSON"))
            .collect();
        (code, parsed)
    }

    #[test]
    fn version_reports_protocol() {
        let (code, out) = lines(&["version"]);
        assert_eq!(code, EXIT_OK);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "result");
        assert_eq!(out[0]["codexUiApi"], API_VERSION);
        assert!(out[0]["version"].as_str().unwrap().contains('.'));
    }

    #[test]
    fn missing_command_and_unknown_flags_are_usage_errors() {
        let (code, out) = lines(&[]);
        assert_eq!(code, EXIT_USAGE);
        assert_eq!(out[0]["type"], "error");
        assert_eq!(out[0]["code"], EXIT_USAGE);

        let (code, out) = lines(&["index", "--data-dir", "X", "--cwd", "Y", "--nope"]);
        assert_eq!(code, EXIT_USAGE);
        assert!(out[0]["message"]
            .as_str()
            .unwrap()
            .contains("未知参数：--nope"));
    }

    #[test]
    fn missing_required_flag_is_reported() {
        let (code, out) = lines(&["list"]);
        assert_eq!(code, EXIT_USAGE);
        assert!(out[0]["message"]
            .as_str()
            .unwrap()
            .contains("--data-dir"));
    }

    #[test]
    fn help_prints_usage_without_error_line() {
        let args = vec!["--help".to_string()];
        let mut buf: Vec<u8> = Vec::new();
        let code = run_with(&args, &mut buf);
        assert_eq!(code, EXIT_OK);
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("用法："), "帮助应输出用法说明：{text}");
        assert!(
            !text.contains("\"type\""),
            "帮助是纯文本，不应出现 JSON 协议行：{text}"
        );
    }

    #[test]
    fn list_on_empty_data_dir_returns_empty_array() {
        let dir = TempDir::new().unwrap();
        let (code, out) = lines(&["list", "--data-dir", &dir.path().to_string_lossy()]);
        assert_eq!(code, EXIT_OK);
        assert_eq!(out[0]["type"], "result");
        assert_eq!(out[0]["kbs"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn index_without_model_reports_model_exit_code() {
        let dir = TempDir::new().unwrap();
        let cwd = dir.path().join("ws");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(cwd.join("a.md"), "# 标题\n\n内容").unwrap();
        let (code, out) = lines(&[
            "index",
            "--data-dir",
            &dir.path().to_string_lossy(),
            "--cwd",
            &cwd.to_string_lossy(),
        ]);
        assert_eq!(code, EXIT_MODEL);
        assert_eq!(out[0]["type"], "error");
        let message = out[0]["message"].as_str().unwrap();
        assert!(
            message.contains("模型文件") || message.contains("onnxruntime.dll"),
            "错误信息应可指导修复：{message}"
        );
    }

    #[test]
    fn index_with_missing_root_is_usage_error() {
        let dir = TempDir::new().unwrap();
        let cwd = dir.path().join("ws");
        std::fs::create_dir_all(&cwd).unwrap();
        let model_dir = dir.path().join("model");
        std::fs::create_dir_all(&model_dir).unwrap();
        let (code, out) = lines(&[
            "index",
            "--data-dir",
            &dir.path().to_string_lossy(),
            "--cwd",
            &cwd.to_string_lossy(),
            "--model-dir",
            &model_dir.to_string_lossy(),
            "--roots",
            &cwd.join("nope").to_string_lossy(),
        ]);
        assert_eq!(code, EXIT_MODEL, "缺模型先于路径校验被拦下");
        assert_eq!(out[0]["type"], "error");
    }

    #[test]
    fn search_on_unindexed_workspace_returns_empty_hits() {
        let dir = TempDir::new().unwrap();
        let (code, out) = lines(&[
            "search",
            "--data-dir",
            &dir.path().to_string_lossy(),
            "--cwd",
            "D:\\没有这个目录",
            "--query",
            "错误码",
        ]);
        assert_eq!(code, EXIT_OK);
        assert_eq!(out[0]["type"], "result");
        assert_eq!(out[0]["hits"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn delete_rejects_path_traversal() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(crate::paths::kbs_dir(dir.path())).unwrap();
        let (code, out) = lines(&[
            "delete",
            "--data-dir",
            &dir.path().to_string_lossy(),
            "--file",
            "../escape.sqlite",
        ]);
        assert_eq!(code, EXIT_INTERNAL);
        assert!(out[0]["message"].as_str().unwrap().contains("非法"));
    }

    #[test]
    fn busy_errors_map_to_dedicated_code() {
        let err = map_engine_error("初始化知识库表失败：database is locked".to_string());
        assert_eq!(err.code, EXIT_BUSY);
        assert!(err.message.contains("请稍后重试"));
    }

    #[test]
    fn flags_accept_inline_values() {
        let args: Vec<String> = ["--data-dir=D:\\kb", "--full", "--roots", "a"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let flags = Flags::parse(&args, &["--data-dir", "--roots"], &["--full"]).unwrap();
        assert_eq!(flags.get("--data-dir"), Some("D:\\kb"));
        assert_eq!(flags.get_all("--roots"), ["a".to_string()]);
        assert!(flags.has("--full"));
    }

    #[test]
    fn default_model_dir_points_to_model_next_to_exe() {
        let flags = Flags::default();
        let resolved = flags.model_dir().unwrap();
        let exe_dir = std::env::current_exe().unwrap();
        let exe_dir = exe_dir.parent().unwrap();
        assert_eq!(resolved, exe_dir.join("model").join(crate::paths::MODEL_ID));
        assert_eq!(crate::paths::model_dir().unwrap(), resolved);
    }

    #[test]
    fn explicit_model_dir_flag_wins() {
        let args: Vec<String> = ["--model-dir", "D:\\models\\zh"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let flags = Flags::parse(&args, &["--model-dir"], &[]).unwrap();
        assert_eq!(
            flags.model_dir().unwrap(),
            std::path::PathBuf::from("D:\\models\\zh")
        );
    }
}
