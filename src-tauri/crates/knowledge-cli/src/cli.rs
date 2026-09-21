//! 命令行协议层：参数解析 + 面向 codex-ui（及 skill）的 NDJSON 输出。
//!
//! stdout 协议（每行一个 JSON 对象，UTF-8）：
//! - 若干 `{"type":"progress","kb":"…","phase":"scan|index|done","processed":n,"total":n,"current":"…"}`
//! - 最后恰好一条 `{"type":"result",…}`（`create` 带 `created/kb/source/file`、`index` 带 `summary`、
//!   `search` 带 `hits`、`list` 带 `kbs`、`delete` 带 `ok`、`version` 带 `version` 与 `codexUiApi`）
//! - 或 `{"type":"error","code":n,"message":"…"}`（中文，可直接展示给用户）
//!
//! stderr 只放人可读日志，codex-ui 只在失败时截取尾部用于诊断。
//!
//! 退出码：0 成功、1 内部错误、2 参数错误、3 模型/ONNX Runtime 未就绪、4 库被占用、
//! 5 保留给"已取消"（由父进程杀进程实现，不主动返回）。

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::atomic::AtomicBool;

use serde_json::json;

use crate::{embed, index, paths, search, store};

/// 协议版本：codex-ui 用它校验 CLI 与自身是否匹配（v2 = create + 按库名操作 + 无路径参数）
pub const API_VERSION: u32 = 2;

pub const EXIT_OK: i32 = 0;
pub const EXIT_INTERNAL: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_MODEL: i32 = 3;
pub const EXIT_BUSY: i32 = 4;
pub const EXIT_CANCELLED: i32 = 5;

const USAGE: &str = "\
codexui-kb —— codex-ui 知识库 CLI

用法：
  codexui-kb create <库名> <目录>              新建库并登记索引来源目录
  codexui-kb index  <库名> [--full]            按已登记来源建库/增量更新
  codexui-kb search <库名> --query <检索词> [--top-k <n>]   检索指定库
  codexui-kb list                              列出所有库
  codexui-kb delete <库名>                     按库名删除（含 -wal/-shm）
  codexui-kb delete --file <库文件名>          删除损坏库（按文件名兜底）
  codexui-kb version                           输出版本与协议版本

库名不能是路径（不含 / \\ : 等字符）；库与工作目录解耦，跨目录共享同一个库时用同一个库名。
数据目录固定 %APPDATA%\\com.codexui.app（库文件在其下的 kbs\\）；模型与 onnxruntime.dll 位于本程序同目录。

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
        "create" => cmd_create(out, rest),
        "index" => cmd_index(out, rest),
        "search" => cmd_search(out, rest),
        "list" => cmd_list(out, rest),
        "delete" => cmd_delete(out, rest),
        other => Err(CliError::usage(format!("未知子命令：{other}\n\n{USAGE}"))),
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

/// `create <库名> <目录>`
fn cmd_create<W: Write>(out: &mut W, rest: &[String]) -> Result<i32, CliError> {
    let args = Args::parse(rest, &[], &[])?;
    let tokens = args.positional();
    let [name, source] = tokens.as_slice() else {
        return Err(CliError::usage(format!(
            "用法：codexui-kb create <库名> <目录>\n\n{USAGE}"
        )));
    };
    let (normalized, display) = paths::normalize_kb_name(name).map_err(CliError::usage)?;
    let source = source.trim();
    if source.is_empty() {
        return Err(CliError::usage("来源目录不能为空"));
    }
    let source_path = std::path::Path::new(source);
    if !source_path.is_dir() {
        return Err(CliError::usage(format!("来源目录不存在或不是目录：{source}")));
    }
    let data_dir = data_dir()?;
    let (store, created) = store::KbStore::create(&data_dir, &normalized, &display, source)
        .map_err(map_engine_error)?;
    let file = store
        .path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    write_line(
        out,
        &json!({
            "type": "result",
            "created": created,
            "kb": store.kb_display(),
            "source": store.source_display(),
            "file": file,
        }),
    )?;
    Ok(EXIT_OK)
}

/// `index <库名> [--full]`
fn cmd_index<W: Write>(out: &mut W, rest: &[String]) -> Result<i32, CliError> {
    let args = Args::parse(rest, &[], &["--full"])?;
    let tokens = args.positional();
    let [name] = tokens.as_slice() else {
        return Err(CliError::usage(format!(
            "用法：codexui-kb index <库名> [--full]\n\n{USAGE}"
        )));
    };
    let (normalized, display) = paths::normalize_kb_name(name).map_err(CliError::usage)?;
    let data_dir = data_dir()?;
    let path = store::require_existing_path(&data_dir, &normalized, &display)
        .map_err(|e| CliError::new(EXIT_USAGE, e))?;
    let opened = store::KbStore::open_existing(&data_dir, &normalized)
        .map_err(map_engine_error)?
        .ok_or_else(|| CliError::new(EXIT_INTERNAL, "打开知识库失败".to_string()))?;
    let source = opened.source_display();
    drop(opened);
    if source.trim().is_empty() {
        return Err(CliError::new(
            EXIT_INTERNAL,
            format!("库「{display}」没有登记来源目录，请重新执行 codexui-kb create \"{display}\" <目录>"),
        ));
    }
    let root = std::path::PathBuf::from(&source);
    if !root.is_dir() {
        return Err(CliError::new(
            EXIT_INTERNAL,
            format!("库「{display}」的来源目录不存在：{source}（请修正目录后重新 create）"),
        ));
    }
    let _ = path;
    let model_dir = paths::model_dir().map_err(|e| CliError::new(EXIT_MODEL, e))?;
    ensure_model(&model_dir)?;

    let cancel = AtomicBool::new(false);
    let mut on_progress = |p: index::IndexProgress| {
        // 父进程（codex-ui）退出会让 stdout 关闭：此时尽快收尾，避免留下孤儿进程
        if write_line(
            out,
            &json!({
                "type": "progress",
                "kb": p.kb,
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
        &normalized,
        &display,
        &root,
        args.has("--full"),
        &cancel,
        &mut on_progress,
    )
    .map_err(map_engine_error)?
    .finalized();
    write_line(out, &json!({ "type": "result", "summary": summary }))?;
    Ok(EXIT_OK)
}

/// `search <库名> --query <检索词> [--top-k <n>]`
fn cmd_search<W: Write>(out: &mut W, rest: &[String]) -> Result<i32, CliError> {
    let args = Args::parse(rest, &["--query", "--top-k"], &[])?;
    let tokens = args.positional();
    let [name] = tokens.as_slice() else {
        return Err(CliError::usage(format!(
            "用法：codexui-kb search <库名> --query <检索词> [--top-k <n>]\n\n{USAGE}"
        )));
    };
    let (normalized, display) = paths::normalize_kb_name(name).map_err(CliError::usage)?;
    let query = args.required("--query")?;
    if query.trim().is_empty() {
        return Err(CliError::usage("--query 不能为空"));
    }
    let top_k = match args.get("--top-k") {
        Some(raw) => raw
            .parse::<usize>()
            .map_err(|_| CliError::usage(format!("--top-k 需要正整数，收到：{raw}")))?,
        None => search::DEFAULT_TOP_K,
    };
    let data_dir = data_dir()?;
    let Some(store) = store::KbStore::open_existing(&data_dir, &normalized).map_err(map_engine_error)?
    else {
        // 库不存在：返回空结果（不报错、不建库），并附上库名便于调用方核对
        write_line(out, &json!({ "type": "result", "kb": display, "hits": [] }))?;
        return Ok(EXIT_OK);
    };
    // 空库（尚未建索引）：无需加载模型即可回答"没有内容"
    let (_, chunk_count) = store.counts();
    if chunk_count == 0 {
        write_line(
            out,
            &json!({ "type": "result", "kb": store.kb_display(), "source": store.source_display(), "hits": [] }),
        )?;
        return Ok(EXIT_OK);
    }
    let model_dir = paths::model_dir().map_err(|e| CliError::new(EXIT_MODEL, e))?;
    ensure_model(&model_dir)?;
    let cache = search::VectorCache::default();
    let hits = search::search(&model_dir, &store, &cache, &query, top_k)
        .map_err(map_engine_error)?;
    write_line(
        out,
        &json!({ "type": "result", "kb": store.kb_display(), "source": store.source_display(), "hits": hits }),
    )?;
    Ok(EXIT_OK)
}

/// `list`
fn cmd_list<W: Write>(out: &mut W, rest: &[String]) -> Result<i32, CliError> {
    let args = Args::parse(rest, &[], &[])?;
    if !args.positional().is_empty() {
        return Err(CliError::usage("list 不接受位置参数"));
    }
    let data_dir = data_dir()?;
    let kbs = store::list_all(&data_dir);
    write_line(out, &json!({ "type": "result", "kbs": kbs }))?;
    Ok(EXIT_OK)
}

/// `delete <库名>` / `delete --file <库文件名>`
fn cmd_delete<W: Write>(out: &mut W, rest: &[String]) -> Result<i32, CliError> {
    let args = Args::parse(rest, &["--file"], &[])?;
    let data_dir = data_dir()?;
    if let Some(file) = args.get("--file") {
        if !args.positional().is_empty() {
            return Err(CliError::usage("delete 不能同时使用库名与 --file"));
        }
        store::delete_file(&data_dir, file).map_err(map_engine_error)?;
        write_line(out, &json!({ "type": "result", "ok": true, "file": file }))?;
        return Ok(EXIT_OK);
    }
    let tokens = args.positional();
    let [name] = tokens.as_slice() else {
        return Err(CliError::usage(format!(
            "用法：codexui-kb delete <库名> | codexui-kb delete --file <库文件名>\n\n{USAGE}"
        )));
    };
    let (normalized, display) = paths::normalize_kb_name(name).map_err(CliError::usage)?;
    let path = store::require_existing_path(&data_dir, &normalized, &display)
        .map_err(|e| CliError::new(EXIT_USAGE, e))?;
    let file = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    store::delete_file(&data_dir, &file).map_err(map_engine_error)?;
    write_line(out, &json!({ "type": "result", "ok": true, "kb": display, "file": file }))?;
    Ok(EXIT_OK)
}

/// 数据目录：固定 `%APPDATA%\com.codexui.app`（库文件在其下的 `kbs\`）
fn data_dir() -> Result<std::path::PathBuf, CliError> {
    paths::data_dir().map_err(|e| CliError::new(EXIT_INTERNAL, e))
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

/// 极简解析器：位置参数 + `--flag value` / `--flag=value` / `--bool`（未知参数即报错）
#[derive(Debug, Default)]
struct Args {
    positional: Vec<String>,
    values: HashMap<String, Vec<String>>,
    bools: HashSet<String>,
}

impl Args {
    fn parse(args: &[String], value_flags: &[&str], bool_flags: &[&str]) -> Result<Self, CliError> {
        let mut parsed = Args::default();
        let mut i = 0usize;
        while i < args.len() {
            let raw = args[i].as_str();
            let name = raw
                .split('=')
                .next()
                .filter(|n| n.starts_with("--") && n.len() > 2)
                .map(str::to_string);
            let Some(name) = name else {
                parsed.positional.push(raw.to_string());
                i += 1;
                continue;
            };
            let inline = raw.contains('=');
            if bool_flags.contains(&name.as_str()) {
                if inline {
                    return Err(CliError::usage(format!("{name} 是开关，不接受取值")));
                }
                parsed.bools.insert(name);
                i += 1;
                continue;
            }
            if !value_flags.contains(&name.as_str()) {
                return Err(CliError::usage(format!("未知参数：{raw}")));
            }
            let value = if inline {
                raw.split_once('=')
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_default()
            } else {
                i += 1;
                args.get(i)
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{name} 缺少取值")))?
            };
            parsed.values.entry(name).or_default().push(value);
            i += 1;
        }
        Ok(parsed)
    }

    fn positional(&self) -> Vec<&str> {
        self.positional.iter().map(|s| s.as_str()).collect()
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values
            .get(name)
            .and_then(|v| v.last())
            .map(|s| s.as_str())
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

    fn has(&self, name: &str) -> bool {
        self.bools.contains(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用临时 APPDATA 跑一段命令（测试间共享环境变量，故内部串行）
    fn run_with_appdata(args: &[&str], appdata: &std::path::Path) -> (i32, Vec<serde_json::Value>) {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let mut buf: Vec<u8> = Vec::new();
        let code = run_with(&owned, &mut buf);
        let _ = appdata;
        let text = String::from_utf8(buf).unwrap();
        let parsed = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .collect();
        (code, parsed)
    }

    /// 环境变量（APPDATA）相关的用例串行执行
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_appdata<T>(dir: &std::path::Path, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("APPDATA");
        std::env::set_var("APPDATA", dir);
        let out = f();
        match previous {
            Some(v) => std::env::set_var("APPDATA", v),
            None => std::env::remove_var("APPDATA"),
        }
        out
    }

    #[test]
    fn version_reports_protocol() {
        let (code, out) = run_with_appdata(&["version"], std::path::Path::new("D:\\tmp"));
        assert_eq!(code, EXIT_OK);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "result");
        assert_eq!(out[0]["codexUiApi"], API_VERSION);
        assert!(out[0]["version"].as_str().unwrap().contains('.'));
    }

    #[test]
    fn unknown_args_and_removed_flags_are_usage_errors() {
        let cases: Vec<Vec<&str>> = vec![
            vec![],
            vec!["index", "--nope"],
            vec!["index", "手册", "--cwd", "D:\\x"],
            vec!["index", "手册", "--roots", "D:\\x"],
            vec!["index", "手册", "--model-dir", "D:\\m"],
            vec!["list", "--data-dir", "D:\\kb"],
            vec!["search", "手册", "--query", "x", "--data-dir", "D:\\kb"],
        ];
        for case in cases {
            let (code, out) = run_with_appdata(&case, std::path::Path::new("D:\\tmp"));
            assert_eq!(code, EXIT_USAGE, "{case:?} 应为参数错误");
            assert_eq!(out[0]["type"], "error");
            assert_eq!(out[0]["code"], EXIT_USAGE);
        }
    }

    #[test]
    fn kb_name_is_validated_in_commands() {
        let (code, out) = run_with_appdata(
            &["create", "D:\\kb", "D:\\src"],
            std::path::Path::new("D:\\tmp"),
        );
        assert_eq!(code, EXIT_USAGE);
        assert!(out[0]["message"].as_str().unwrap().contains("库名"));
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
    fn create_then_list_search_delete_roundtrip() {
        let appdata = tempfile::TempDir::new().unwrap();
        let source = tempfile::TempDir::new().unwrap();
        std::fs::write(source.path().join("a.md"), "# 标题\n\n内容").unwrap();
        let source_s = source.path().to_string_lossy().to_string();

        with_appdata(appdata.path(), || {
            // 新建
            let (code, out) = run_with_appdata(&["create", "手册", &source_s], appdata.path());
            assert_eq!(code, EXIT_OK, "{out:?}");
            assert_eq!(out[0]["created"], true);
            assert_eq!(out[0]["kb"], "手册");

            // 幂等
            let (code, out) = run_with_appdata(&["create", "手册", &source_s], appdata.path());
            assert_eq!(code, EXIT_OK);
            assert_eq!(out[0]["created"], false);

            // 同名不同来源 → 报错
            let other = tempfile::TempDir::new().unwrap();
            let (code, out) = run_with_appdata(
                &["create", "手册", &other.path().to_string_lossy()],
                appdata.path(),
            );
            assert_eq!(code, EXIT_INTERNAL);
            assert!(out[0]["message"].as_str().unwrap().contains("来源"));

            // list 能列出
            let (code, out) = run_with_appdata(&["list"], appdata.path());
            assert_eq!(code, EXIT_OK);
            let rows = out[0]["kbs"].as_array().unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["kb"], "手册");
            assert_eq!(rows[0]["source"], source_s);
            assert!(rows[0].get("cwd").is_none(), "不应再有 cwd 字段");

            // search（库存在但未建索引）→ 空 hits，不报错
            let (code, out) =
                run_with_appdata(&["search", "手册", "--query", "错误码"], appdata.path());
            assert_eq!(code, EXIT_OK);
            assert_eq!(out[0]["hits"].as_array().unwrap().len(), 0);

            // search 未建库 → 空 hits
            let (code, out) =
                run_with_appdata(&["search", "没有这个库", "--query", "x"], appdata.path());
            assert_eq!(code, EXIT_OK);
            assert_eq!(out[0]["hits"].as_array().unwrap().len(), 0);

            // delete 按库名
            let (code, _) = run_with_appdata(&["delete", "手册"], appdata.path());
            assert_eq!(code, EXIT_OK);
            let (_, out) = run_with_appdata(&["list"], appdata.path());
            assert_eq!(out[0]["kbs"].as_array().unwrap().len(), 0);
        });
    }

    #[test]
    fn create_rejects_missing_source_directory() {
        let appdata = tempfile::TempDir::new().unwrap();
        with_appdata(appdata.path(), || {
            let missing = appdata.path().join("nope");
            let (code, out) = run_with_appdata(
                &["create", "手册", &missing.to_string_lossy()],
                appdata.path(),
            );
            assert_eq!(code, EXIT_USAGE);
            assert!(out[0]["message"].as_str().unwrap().contains("不存在"));
        });
    }

    #[test]
    fn index_requires_existing_kb_and_model() {
        let appdata = tempfile::TempDir::new().unwrap();
        let source = tempfile::TempDir::new().unwrap();
        let source_s = source.path().to_string_lossy().to_string();
        with_appdata(appdata.path(), || {
            // 未 create → 提示先 create
            let (code, out) = run_with_appdata(&["index", "手册"], appdata.path());
            assert_eq!(code, EXIT_USAGE);
            assert!(out[0]["message"].as_str().unwrap().contains("create"));

            let (code, _) =
                run_with_appdata(&["create", "手册", &source_s], appdata.path());
            assert_eq!(code, EXIT_OK);
            // 已 create 但本机无模型 → 退出码 3（模型未就绪）
            let (code, out) = run_with_appdata(&["index", "手册"], appdata.path());
            assert_eq!(code, EXIT_MODEL);
            assert_eq!(out[0]["type"], "error");
        });
    }

    #[test]
    fn delete_by_file_rejects_path_traversal() {
        let appdata = tempfile::TempDir::new().unwrap();
        with_appdata(appdata.path(), || {
            let (code, out) =
                run_with_appdata(&["delete", "--file", "../escape.sqlite"], appdata.path());
            assert_eq!(code, EXIT_INTERNAL);
            assert!(out[0]["message"].as_str().unwrap().contains("非法"));
        });
    }

    #[test]
    fn missing_appdata_reports_internal_error() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("APPDATA");
        std::env::remove_var("APPDATA");
        let (code, out) = run_with_appdata(&["list"], std::path::Path::new("X"));
        match previous {
            Some(v) => std::env::set_var("APPDATA", v),
            None => std::env::remove_var("APPDATA"),
        }
        assert_eq!(code, EXIT_INTERNAL);
        assert!(out[0]["message"].as_str().unwrap().contains("APPDATA"));
    }

    #[test]
    fn busy_errors_map_to_dedicated_code() {
        let err = map_engine_error("初始化知识库表失败：database is locked".to_string());
        assert_eq!(err.code, EXIT_BUSY);
        assert!(err.message.contains("请稍后重试"));
    }

    #[test]
    fn args_accept_inline_values_and_positionals() {
        let args: Vec<String> = ["手册", "--top-k=5", "--query", "错误码"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let parsed = Args::parse(&args, &["--query", "--top-k"], &[]).unwrap();
        assert_eq!(parsed.positional(), vec!["手册"]);
        assert_eq!(parsed.get("--top-k"), Some("5"));
        assert_eq!(parsed.required("--query").unwrap(), "错误码");
    }

    #[test]
    fn model_dir_is_next_to_exe_and_not_configurable() {
        let expected = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("model")
            .join(crate::paths::MODEL_ID);
        assert_eq!(crate::paths::model_dir().unwrap(), expected);
    }
}
