use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::codex::app_server::app_exe_dir;
use crate::codex::file_icon::{icon_data_uri, icon_data_uri_for_ext, DEFAULT_ICON_SIZE};
use crate::codex::path_util::{
    clean_path, is_inside_path, norm_key, rel_path_of as shared_rel_path_of,
};
use crate::codex::util::{resolve_workspace_dir, spawn_blocking_timeout, BlockingError};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

/// 会话资源条目（camelCase 序列化，供前端直接使用）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    /// 相对 root 的路径（. 表示根）
    pub rel_path: String,
    pub is_dir: bool,
    /// 文件字节数；目录为 None
    pub size: Option<u64>,
    pub modified_at_ms: i64,
    pub created_at_ms: i64,
    /// 目录的直接可见子项数（文件为 None）
    pub child_count: Option<u64>,
}

/// 捆绑 rg 可用状态（camelCase 序列化）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RgStatus {
    pub available: bool,
    pub path: Option<String>,
}

/// rg 内容命中：每个文件只取首个命中行
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RgHit {
    pub entry: FsEntry,
    pub line_number: u64,
    pub line_text: String,
}

/// rg 内容搜索结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RgSearchResult {
    pub available: bool,
    pub hits: Vec<RgHit>,
}

/// 文件监听句柄（sender 被 drop 时防抖任务随 rx 关闭退出）
pub struct WatchHandle {
    _watcher: RecommendedWatcher,
    _sender: tokio::sync::mpsc::Sender<PathBuf>,
    root: PathBuf,
}

pub struct FsWatcherState(pub Mutex<Option<WatchHandle>>);

fn is_dot_dir(name: &str) -> bool {
    name.starts_with('.') && name.len() > 1
}

/// 需要无条件忽略的目录：点目录或 node_modules（任意深度，大小写不敏感）
fn is_noise_dir(name: &str) -> bool {
    is_dot_dir(name) || name.eq_ignore_ascii_case("node_modules")
}

/// 相对 root 的相对路径（. 表示根）；root 外时回退为清洗后的全路径（正斜杠）。
/// 比较/相对化统一走 path_util::rel_path_of（大小写不敏感、兼容 `\\?\` 前缀）。
fn rel_path_of(root: &Path, path: &Path) -> String {
    shared_rel_path_of(root, path).unwrap_or_else(|| clean_path(path).replace('\\', "/"))
}

/// target 必须位于 root 内（规范化后，Windows 大小写不敏感）。
/// 相对路径按 root 解析（工作区外文件以父目录为根、传文件名的场景），
/// 绝对路径原样使用。
fn ensure_inside(root: &Path, target: &Path) -> Result<PathBuf, String> {
    let root_c = root
        .canonicalize()
        .map_err(|e| format!("无法访问工作目录 {}: {e}", clean_path(root)))?;
    let joined = if target.is_absolute() {
        target.to_path_buf()
    } else {
        root.join(target)
    };
    let target_c = joined
        .canonicalize()
        .map_err(|e| format!("无法访问路径 {}: {e}", clean_path(target)))?;
    if is_inside_path(&root_c, &target_c) {
        Ok(target_c)
    } else {
        Err(format!("路径越界: {}", clean_path(target)))
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.trim() != name {
        return Err("名称不能为空或包含首尾空格".into());
    }
    if name == "." || name == ".." {
        return Err("非法名称".into());
    }
    if name.ends_with('.') {
        return Err("Windows 名称不能以点结尾".into());
    }
    if name.contains('\\')
        || name.contains('/')
        || name
            .chars()
            .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err("名称包含非法字符".into());
    }
    Ok(())
}

fn ts_ms(t: Option<SystemTime>) -> i64 {
    t.and_then(|x| x.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 目录直接可见子项数（过滤点目录）
fn visible_child_count(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    !(is_dir && is_noise_dir(&name))
                })
                .count() as u64
        })
        .unwrap_or(0)
}

/// 条目显示名：普通路径取末段文件名；根目录（无文件名）回退到 clean_path，
/// 避免 Windows canonicalize 产生的 `\\?\` 前缀泄漏到界面。
fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| clean_path(path))
}

fn entry_from_path(root: &Path, path: &Path) -> Result<FsEntry, String> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("读取元信息失败 {}: {e}", clean_path(path)))?;
    let is_dir = meta.file_type().is_dir();
    let name = display_name(path);
    let rel_path = rel_path_of(root, path);
    Ok(FsEntry {
        name,
        path: clean_path(path),
        rel_path,
        is_dir,
        size: if is_dir { None } else { Some(meta.len()) },
        modified_at_ms: ts_ms(meta.modified().ok()),
        created_at_ms: ts_ms(meta.created().ok()),
        child_count: if is_dir {
            Some(visible_child_count(path))
        } else {
            None
        },
    })
}

fn sort_entries(a: &FsEntry, b: &FsEntry) -> std::cmp::Ordering {
    match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a
            .name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name)),
    }
}

fn list_impl(root: &Path, dir: &Path) -> Result<Vec<FsEntry>, String> {
    let dir_c = ensure_inside(root, dir)?;
    if !dir_c.is_dir() {
        return Err(format!("不是目录: {}", clean_path(dir)));
    }
    let mut entries = Vec::new();
    let rd =
        std::fs::read_dir(&dir_c).map_err(|e| format!("读取目录失败 {}: {e}", clean_path(dir)))?;
    for item in rd.flatten() {
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        let is_dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir && is_noise_dir(&name) {
            continue;
        }
        if let Ok(e) = entry_from_path(root, &path) {
            entries.push(e);
        }
    }
    entries.sort_by(sort_entries);
    Ok(entries)
}

fn search_impl(root: &Path, query: &str, limit: usize) -> Result<Vec<FsEntry>, String> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let max = limit.min(500);
    // 遍历上限：防止超大目录下长时间占用阻塞线程
    const MAX_VISITED_DIRS: usize = 50_000;
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0usize;
    while let Some(dir) = stack.pop() {
        visited += 1;
        if out.len() >= max || visited > MAX_VISITED_DIRS {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in rd.flatten() {
            let path = item.path();
            let name = item.file_name().to_string_lossy().into_owned();
            let is_dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if is_noise_dir(&name) {
                    continue;
                }
                stack.push(path.clone());
            }
            if name.to_lowercase().contains(&q) {
                if let Ok(e) = entry_from_path(root, &path) {
                    out.push(e);
                    if out.len() >= max {
                        break;
                    }
                }
            }
        }
    }
    out.sort_by(sort_entries);
    Ok(out)
}

/// <应用目录>/bin/rg.exe，存在才返回（纯函数，便于测试）
fn rg_exe_in(app_dir: &Path) -> Option<PathBuf> {
    let p = app_dir.join("bin").join("rg.exe");
    p.is_file().then_some(p)
}

/// 当前应用目录下捆绑的 rg；无法定位应用目录或文件缺失时返回 None。
fn bundled_rg_exe() -> Option<PathBuf> {
    let dir = app_exe_dir()?;
    rg_exe_in(&dir)
}

/// Windows GUI 应用下隐藏 rg 控制台窗口
fn rg_command(rg: &Path) -> Command {
    let mut cmd = Command::new(rg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// 资源面板内容搜索参数：普通文本、忽略大小写、每文件首个命中、
/// 跳过 node_modules 与超大文件；`--` 保证以 `-` 开头的查询不被当作选项。
fn rg_args(query: &str, root: &Path) -> Vec<String> {
    vec![
        "--json".into(),
        "--fixed-strings".into(),
        "--ignore-case".into(),
        "--max-count".into(),
        "1".into(),
        "--max-filesize".into(),
        "2M".into(),
        "--no-messages".into(),
        "--glob".into(),
        "!**/node_modules/**".into(),
        "--".into(),
        query.into(),
        root.to_string_lossy().into_owned(),
    ]
}

/// 命中行摘要：去掉首尾空白，超过上限时截断并追加省略号。
fn truncate_snippet(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// 解析 `rg --json` 输出中的 match 事件；跳过畸形行、root 外路径与重复文件。
fn parse_rg_json(stdout: &str, root: &Path, limit: usize) -> Vec<RgHit> {
    const MAX_HITS: usize = 200;
    const MAX_SNIPPET_CHARS: usize = 200;
    let max = limit.min(MAX_HITS);
    let mut hits = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in stdout.lines() {
        if hits.len() >= max {
            break;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(|t| t.as_str()) != Some("match") {
            continue;
        }
        let Some(data) = value.get("data") else {
            continue;
        };
        let Some(path_text) = data.pointer("/path/text").and_then(|p| p.as_str()) else {
            continue;
        };
        let raw_path = PathBuf::from(path_text);
        let raw_path = if raw_path.is_absolute() {
            raw_path
        } else {
            root.join(raw_path)
        };
        let Ok(target) = ensure_inside(root, &raw_path) else {
            continue;
        };
        if !target.is_file() {
            continue;
        }
        if !seen.insert(norm_key(&target)) {
            continue;
        }
        let Ok(entry) = entry_from_path(root, &target) else {
            continue;
        };
        let line_number = data
            .get("line_number")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let line_text = data
            .pointer("/lines/text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim();
        hits.push(RgHit {
            entry,
            line_number,
            line_text: truncate_snippet(line_text, MAX_SNIPPET_CHARS),
        });
    }
    hits.sort_by(|a, b| {
        a.entry
            .rel_path
            .to_lowercase()
            .cmp(&b.entry.rel_path.to_lowercase())
            .then_with(|| a.entry.rel_path.cmp(&b.entry.rel_path))
    });
    hits
}

/// 执行一次 rg 内容搜索；rg 缺失/执行失败/超时均由调用方转为静默空结果。
fn search_rg_impl(root: &Path, query: &str, limit: usize, rg: Option<&Path>) -> RgSearchResult {
    let q = query.trim();
    let Some(rg) = rg else {
        return RgSearchResult {
            available: false,
            hits: Vec::new(),
        };
    };
    if q.is_empty() {
        return RgSearchResult {
            available: true,
            hits: Vec::new(),
        };
    }
    let output = match rg_command(rg).args(rg_args(q, root)).output() {
        Ok(out) => out,
        Err(_) => {
            return RgSearchResult {
                available: true,
                hits: Vec::new(),
            }
        }
    };
    // rg: 0 = 有匹配，1 = 无匹配，其余视为执行失败（静默回退文件名结果）
    if !matches!(output.status.code(), Some(0) | Some(1)) {
        return RgSearchResult {
            available: true,
            hits: Vec::new(),
        };
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    RgSearchResult {
        available: true,
        hits: parse_rg_json(&stdout, root, limit),
    }
}

fn metadata_impl(root: &Path, path: &Path) -> Result<FsEntry, String> {
    let c = ensure_inside(root, path)?;
    entry_from_path(root, &c)
}

fn rename_impl(root: &Path, path: &Path, new_name: &str) -> Result<FsEntry, String> {
    validate_name(new_name)?;
    let c = ensure_inside(root, path)?;
    let root_c = root
        .canonicalize()
        .map_err(|e| format!("无法访问工作目录 {}: {e}", clean_path(root)))?;
    if norm_key(&c) == norm_key(&root_c) {
        return Err("不能重命名工作目录".into());
    }
    let parent = c.parent().ok_or_else(|| "无法确定父目录".to_string())?;
    let dest = parent.join(new_name);
    if dest.exists() {
        return Err(format!("已存在同名文件或目录: {new_name}"));
    }
    std::fs::rename(&c, &dest).map_err(|e| format!("重命名失败 {}: {e}", clean_path(&c)))?;
    entry_from_path(root, &dest)
}

fn delete_impl(root: &Path, path: &Path) -> Result<(), String> {
    let c = ensure_inside(root, path)?;
    let root_c = root
        .canonicalize()
        .map_err(|e| format!("无法访问工作目录 {}: {e}", clean_path(root)))?;
    if norm_key(&c) == norm_key(&root_c) {
        return Err("不能删除工作目录".into());
    }
    let meta = std::fs::symlink_metadata(&c)
        .map_err(|e| format!("读取元信息失败 {}: {e}", clean_path(&c)))?;
    if meta.file_type().is_dir() {
        std::fs::remove_dir_all(&c).map_err(|e| format!("删除目录失败 {}: {e}", clean_path(&c)))?;
    } else {
        std::fs::remove_file(&c).map_err(|e| format!("删除文件失败 {}: {e}", clean_path(&c)))?;
    }
    Ok(())
}

fn copy_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(src)
        .map_err(|e| format!("读取元信息失败 {}: {e}", clean_path(src)))?;
    if meta.file_type().is_dir() {
        std::fs::create_dir_all(dst)
            .map_err(|e| format!("创建目录失败 {}: {e}", clean_path(dst)))?;
        let rd =
            std::fs::read_dir(src).map_err(|e| format!("读取目录失败 {}: {e}", clean_path(src)))?;
        for item in rd.flatten() {
            copy_recursive(&item.path(), &dst.join(item.file_name()))?;
        }
    } else {
        std::fs::copy(src, dst).map_err(|e| format!("复制失败 {}: {e}", clean_path(src)))?;
    }
    Ok(())
}

fn copy_impl(root: &Path, src: &Path, dest_dir: &Path) -> Result<FsEntry, String> {
    let src_c = ensure_inside(root, src)?;
    let dest_c = ensure_inside(root, dest_dir)?;
    if !dest_c.is_dir() {
        return Err("目标必须是目录".into());
    }
    if is_inside_path(&src_c, &dest_c) {
        return Err("不能把目录复制到自身或子目录".into());
    }
    let name = src_c
        .file_name()
        .ok_or_else(|| "无法确定源名称".to_string())?
        .to_string_lossy()
        .into_owned();
    let target = dest_c.join(&name);
    if target.exists() {
        return Err(format!("目标已存在: {}", clean_path(&target)));
    }
    copy_recursive(&src_c, &target)?;
    entry_from_path(root, &target)
}

fn paste_impl(root: &Path, dest_dir: &Path, sources: &[String]) -> Result<Vec<FsEntry>, String> {
    let dest_c = ensure_inside(root, dest_dir)?;
    if !dest_c.is_dir() {
        return Err("目标必须是目录".into());
    }
    let mut out = Vec::new();
    for s in sources {
        let src_p = PathBuf::from(s);
        let src_c = src_p
            .canonicalize()
            .map_err(|e| format!("无法访问源 {}: {e}", clean_path(&src_p)))?;
        if is_inside_path(&src_c, &dest_c) {
            return Err(format!(
                "不能把「{}」粘贴到自身或子目录",
                clean_path(&src_c)
            ));
        }
        let name = src_c
            .file_name()
            .ok_or_else(|| "无法确定源名称".to_string())?
            .to_string_lossy()
            .into_owned();
        let target = dest_c.join(&name);
        if target.exists() {
            return Err(format!("目标已存在: {}", clean_path(&target)));
        }
        copy_recursive(&src_c, &target)?;
        out.push(entry_from_path(root, &target)?);
    }
    Ok(out)
}

/// 移动：优先同卷 rename；跨盘（CrossesDevices）降级为递归复制 + 删除源。
/// 禁止移动根、移动到自身或子目录；目标目录必须存在且同名不冲突。
fn move_impl(root: &Path, src: &Path, dest_dir: &Path) -> Result<FsEntry, String> {
    let src_c = ensure_inside(root, src)?;
    let dest_c = ensure_inside(root, dest_dir)?;
    let root_c = root
        .canonicalize()
        .map_err(|e| format!("无法访问工作目录 {}: {e}", clean_path(root)))?;
    if norm_key(&src_c) == norm_key(&root_c) {
        return Err("不能移动工作目录".into());
    }
    if !dest_c.is_dir() {
        return Err("目标必须是目录".into());
    }
    if norm_key(&src_c) == norm_key(&dest_c) {
        return Err("不能移动到自身".into());
    }
    if is_inside_path(&src_c, &dest_c) {
        return Err("不能移动到自身或子目录".into());
    }
    let name = src_c
        .file_name()
        .ok_or_else(|| "无法确定源名称".to_string())?
        .to_string_lossy()
        .into_owned();
    let target = dest_c.join(&name);
    if target.exists() {
        return Err(format!("目标已存在: {}", clean_path(&target)));
    }
    match std::fs::rename(&src_c, &target) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_recursive(&src_c, &target)?;
            if src_c.is_dir() {
                std::fs::remove_dir_all(&src_c).map_err(|e| {
                    format!(
                        "移动失败（跨盘复制后删除源目录） {}: {e}",
                        clean_path(&src_c)
                    )
                })?;
            } else {
                std::fs::remove_file(&src_c).map_err(|e| {
                    format!(
                        "移动失败（跨盘复制后删除源文件） {}: {e}",
                        clean_path(&src_c)
                    )
                })?;
            }
        }
        Err(e) => return Err(format!("移动失败 {}: {e}", clean_path(&src_c))),
    }
    entry_from_path(root, &target)
}

#[tauri::command]
pub async fn session_fs_list(workspace: String, dir: String) -> Result<Vec<FsEntry>, String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        list_impl(&root_p, Path::new(&dir))
    })
    .await
}

#[tauri::command]
pub async fn session_fs_search(
    workspace: String,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<FsEntry>, String> {
    run_blocking(120, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        search_impl(&root_p, &query, limit.unwrap_or(200))
    })
    .await
}

#[tauri::command]
pub async fn session_fs_rg_status() -> Result<RgStatus, String> {
    let path = bundled_rg_exe();
    Ok(RgStatus {
        available: path.is_some(),
        path: path.map(|p| clean_path(&p)),
    })
}

#[tauri::command]
pub async fn session_fs_search_rg(
    workspace: String,
    query: String,
    limit: Option<usize>,
) -> Result<RgSearchResult, String> {
    let rg = bundled_rg_exe();
    run_blocking(30, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        Ok(search_rg_impl(
            &root_p,
            &query,
            limit.unwrap_or(100),
            rg.as_deref(),
        ))
    })
    .await
}

#[tauri::command]
pub async fn session_fs_metadata(workspace: String, path: String) -> Result<FsEntry, String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        metadata_impl(&root_p, Path::new(&path))
    })
    .await
}

#[tauri::command]
pub async fn session_fs_rename(
    workspace: String,
    path: String,
    new_name: String,
) -> Result<FsEntry, String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        rename_impl(&root_p, Path::new(&path), &new_name)
    })
    .await
}

#[tauri::command]
pub async fn session_fs_delete(workspace: String, path: String) -> Result<(), String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        delete_impl(&root_p, Path::new(&path))
    })
    .await
}

#[tauri::command]
pub async fn session_fs_copy(
    workspace: String,
    src: String,
    dest_dir: String,
) -> Result<FsEntry, String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        copy_impl(&root_p, Path::new(&src), Path::new(&dest_dir))
    })
    .await
}

#[tauri::command]
pub async fn session_fs_paste(
    workspace: String,
    dest_dir: String,
    sources: Vec<String>,
) -> Result<Vec<FsEntry>, String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        paste_impl(&root_p, Path::new(&dest_dir), &sources)
    })
    .await
}

#[tauri::command]
pub async fn session_fs_move(
    workspace: String,
    src: String,
    dest_dir: String,
) -> Result<FsEntry, String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        move_impl(&root_p, Path::new(&src), Path::new(&dest_dir))
    })
    .await
}

/// 文本编辑器最大读写字节数（5 MiB）
const MAX_EDIT_BYTES: u64 = 5 * 1024 * 1024;

/// 文本文件内容（供文本编辑器使用；camelCase 序列化）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextFileContent {
    pub content: String,
    /// 内容是否为合法 UTF-8；false 表示仅 lossy 展示，应只读
    pub valid_utf8: bool,
    pub byte_size: u64,
}

/// 单文件内容读取（文本编辑器用）：路径包含校验、仅文件、大小上限、
/// UTF-8 合法性标记（非法编码以 lossy 展示，由前端置为只读）
fn read_impl(root: &Path, path: &Path) -> Result<TextFileContent, String> {
    let target = ensure_inside(root, path)?;
    let meta = std::fs::metadata(&target)
        .map_err(|e| format!("无法读取文件 {}: {e}", clean_path(&target)))?;
    if !meta.is_file() {
        return Err(format!("不是文件: {}", clean_path(&target)));
    }
    if meta.len() > MAX_EDIT_BYTES {
        return Err("文件过大，暂不支持编辑".into());
    }
    let bytes =
        std::fs::read(&target).map_err(|e| format!("无法读取文件 {}: {e}", clean_path(&target)))?;
    let (content, valid_utf8) = match String::from_utf8(bytes) {
        Ok(s) => (s, true),
        Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), false),
    };
    Ok(TextFileContent {
        content,
        valid_utf8,
        byte_size: meta.len(),
    })
}

/// 在阻塞线程中执行同步文件操作，带超时（秒）；错误文案与旧实现保持一致。
async fn run_blocking<T: Send + 'static>(
    timeout_secs: u64,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    match spawn_blocking_timeout(timeout_secs, f).await {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => Err(e),
        Err(BlockingError::Timeout(secs)) => Err(format!("文件操作超时（{secs} 秒）")),
        Err(BlockingError::Join(e)) => Err(format!("文件操作任务异常: {e}")),
    }
}

#[tauri::command]
pub async fn session_fs_read(workspace: String, path: String) -> Result<TextFileContent, String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        read_impl(&root_p, Path::new(&path))
    })
    .await
}

/// 单文件内容写入（文本编辑器用）：路径包含校验、仅文件、大小上限、
/// NUL 检测，UTF-8 直写；返回写入后的文件条目
fn write_impl(root: &Path, path: &Path, content: &str) -> Result<FsEntry, String> {
    let target = ensure_inside(root, path)?;
    let meta = std::fs::metadata(&target)
        .map_err(|e| format!("无法写入文件 {}: {e}", clean_path(&target)))?;
    if !meta.is_file() {
        return Err(format!("不是文件: {}", clean_path(&target)));
    }
    let bytes = content.as_bytes();
    if bytes.len() as u64 > MAX_EDIT_BYTES {
        return Err("文件过大，暂不支持保存".into());
    }
    if bytes.contains(&0) {
        return Err("内容包含二进制数据，已拒绝保存".into());
    }
    std::fs::write(&target, bytes)
        .map_err(|e| format!("写入文件失败 {}: {e}", clean_path(&target)))?;
    entry_from_path(root, &target)
}

#[tauri::command]
pub async fn session_fs_write(
    workspace: String,
    path: String,
    content: String,
) -> Result<FsEntry, String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        write_impl(&root_p, Path::new(&path), &content)
    })
    .await
}

/// 二进制内容写入（.docx 等富文本编辑用）：base64 解码后直写，
/// 守卫与 write_impl 一致（路径包含校验、仅文件），大小上限与
/// 二进制读取一致（MAX_PREVIEW_BYTES）。
fn write_bytes_impl(
    root: &Path,
    path: &Path,
    content: &str,
    max_bytes: u64,
) -> Result<FsEntry, String> {
    // base64 长度约 4/3 于原始字节：先按文本长度预检，避免超大负载先解码再拒绝
    if content.len() as u64 > max_bytes * 4 / 3 + 4 {
        return Err("文件过大，暂不支持保存".into());
    }
    let bytes = STANDARD
        .decode(content)
        .map_err(|_| "内容不是合法的 base64".to_string())?;
    if bytes.len() as u64 > max_bytes {
        return Err("文件过大，暂不支持保存".into());
    }
    let target = ensure_inside(root, path)?;
    let meta = std::fs::metadata(&target)
        .map_err(|e| format!("无法写入文件 {}: {e}", clean_path(&target)))?;
    if !meta.is_file() {
        return Err(format!("不是文件: {}", clean_path(&target)));
    }
    std::fs::write(&target, &bytes)
        .map_err(|e| format!("写入文件失败 {}: {e}", clean_path(&target)))?;
    entry_from_path(root, &target)
}

#[tauri::command]
pub async fn session_fs_write_bytes(
    workspace: String,
    path: String,
    content: String,
) -> Result<FsEntry, String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        write_bytes_impl(&root_p, Path::new(&path), &content, MAX_PREVIEW_BYTES)
    })
    .await
}

/// 新建文本文件：目标必须为根内目录，自动生成唯一名
/// （新建文本文件.txt，冲突时追加 (2)、(3)…），创建空文件并返回条目
fn create_file_impl(root: &Path, dir: &Path) -> Result<FsEntry, String> {
    let dir_c = ensure_inside(root, dir)?;
    if !dir_c.is_dir() {
        return Err("目标必须是目录".into());
    }
    const BASE: &str = "新建文本文件";
    let mut name = format!("{BASE}.txt");
    let mut n = 2;
    while dir_c.join(&name).exists() {
        name = format!("{BASE} ({n}).txt");
        n += 1;
    }
    let target = dir_c.join(&name);
    std::fs::write(&target, "")
        .map_err(|e| format!("创建文件失败 {}: {e}", clean_path(&target)))?;
    entry_from_path(root, &target)
}

/// 新建文件夹：目标必须为根内目录，自动生成唯一名
/// （新建文件夹，冲突时追加 (2)、(3)…），创建空目录并返回条目
fn create_dir_impl(root: &Path, dir: &Path) -> Result<FsEntry, String> {
    let dir_c = ensure_inside(root, dir)?;
    if !dir_c.is_dir() {
        return Err("目标必须是目录".into());
    }
    const BASE: &str = "新建文件夹";
    let mut name = BASE.to_string();
    let mut n = 2;
    while dir_c.join(&name).exists() {
        name = format!("{BASE} ({n})");
        n += 1;
    }
    let target = dir_c.join(&name);
    std::fs::create_dir(&target)
        .map_err(|e| format!("创建文件夹失败 {}: {e}", clean_path(&target)))?;
    entry_from_path(root, &target)
}

#[tauri::command]
pub async fn session_fs_create_file(workspace: String, dir: String) -> Result<FsEntry, String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        create_file_impl(&root_p, Path::new(&dir))
    })
    .await
}

#[tauri::command]
pub async fn session_fs_create_dir(workspace: String, dir: String) -> Result<FsEntry, String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        create_dir_impl(&root_p, Path::new(&dir))
    })
    .await
}

/// 文本探测采样字节数（与 git 二进制判定窗口一致，覆盖常见文件头）
const TEXT_PROBE_BYTES: usize = 8000;

/// 内容级文本判定：前 8000 字节内出现 NUL → 二进制；无 NUL 且为空或
/// 合法 UTF-8（仅采样末尾多字节序列被截断视为合法）→ 文本。
pub(crate) fn looks_text(data: &[u8]) -> bool {
    if data.contains(&0) {
        return false;
    }
    if data.is_empty() {
        return true;
    }
    match std::str::from_utf8(data) {
        Ok(_) => true,
        // error_len() == None 表示仅末尾存在未完成的多字节序列（采样截断），仍视为文本
        Err(e) => e.error_len().is_none(),
    }
}

/// 打开前探测文件内容是否为文本：路径包含校验、仅文件、采样前 8000 字节
fn probe_text_impl(root: &Path, path: &Path) -> Result<bool, String> {
    let target = ensure_inside(root, path)?;
    let meta = std::fs::metadata(&target)
        .map_err(|e| format!("无法读取文件 {}: {e}", clean_path(&target)))?;
    if !meta.is_file() {
        return Err(format!("不是文件: {}", clean_path(&target)));
    }
    let mut buf = vec![0u8; TEXT_PROBE_BYTES];
    let mut file = std::fs::File::open(&target)
        .map_err(|e| format!("无法打开文件 {}: {e}", clean_path(&target)))?;
    let mut n = 0;
    while n < buf.len() {
        let read = file
            .read(&mut buf[n..])
            .map_err(|e| format!("无法读取文件 {}: {e}", clean_path(&target)))?;
        if read == 0 {
            break;
        }
        n += read;
    }
    Ok(looks_text(&buf[..n]))
}

#[tauri::command]
pub async fn session_fs_probe_text(workspace: String, path: String) -> Result<bool, String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        probe_text_impl(&root_p, Path::new(&path))
    })
    .await
}

/// 二进制预览读取上限（PDF 预览用；图像预览走 asset 协议不经过此命令）
const MAX_PREVIEW_BYTES: u64 = 50 * 1024 * 1024;

/// 二进制文件读取（PDF/XLSX/DOCX 预览与编辑用）：路径包含校验、仅文件、大小上限（参数化便于测试）。
/// 直接返回原始字节：Tauri v2 IPC 对 `Vec<u8>` 返回值走原始二进制通道（InvokeResponseBody::Raw），
/// 前端 invoke 直接拿到 ArrayBuffer，无需 base64 中转。
fn read_bytes_impl(root: &Path, path: &Path, max_bytes: u64) -> Result<Vec<u8>, String> {
    let target = ensure_inside(root, path)?;
    let meta = std::fs::metadata(&target)
        .map_err(|e| format!("无法读取文件 {}: {e}", clean_path(&target)))?;
    if !meta.is_file() {
        return Err(format!("不是文件: {}", clean_path(&target)));
    }
    if meta.len() > max_bytes {
        return Err(format!(
            "文件过大，暂不支持预览（上限 {} MiB）",
            max_bytes / (1024 * 1024)
        ));
    }
    let bytes =
        std::fs::read(&target).map_err(|e| format!("无法读取文件 {}: {e}", clean_path(&target)))?;
    Ok(bytes)
}

#[tauri::command]
pub async fn session_fs_read_bytes(workspace: String, path: String) -> Result<Vec<u8>, String> {
    run_blocking(120, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        read_bytes_impl(&root_p, Path::new(&path), MAX_PREVIEW_BYTES)
    })
    .await
}

/// 图标请求：path 为会话内文件绝对路径（仅文件；文件夹图标不在范围）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconRequest {
    pub path: String,
}

/// 图标结果：data_uri 为空表示该条目取不到系统图标（前端回退 SVG）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IconResult {
    pub path: String,
    pub data_uri: Option<String>,
}

/// 批量取系统图标：逐条路径包含校验；单条越界/失效路径返回 data_uri=None，
/// 不中断整批（root 本身无效才整体失败）。
fn icons_impl(root: &Path, requests: &[IconRequest], size: u32) -> Vec<IconResult> {
    requests
        .iter()
        .map(|r| {
            let data_uri = ensure_inside(root, Path::new(&r.path))
                .ok()
                // SHGetFileInfo 无法处理 canonicalize 的 \\?\ 前缀路径，先清洗为标准路径
                .and_then(|p| {
                    icon_data_uri(Path::new(&clean_path(&p)), size)
                        .ok()
                        .flatten()
                });
            IconResult {
                path: r.path.clone(),
                data_uri,
            }
        })
        .collect()
}

#[tauri::command]
pub async fn session_fs_icons(
    workspace: String,
    requests: Vec<IconRequest>,
    size: Option<u32>,
) -> Result<Vec<IconResult>, String> {
    run_blocking(60, move || {
        let root_p = resolve_workspace_dir(&workspace)?;
        Ok(icons_impl(
            &root_p,
            &requests,
            size.unwrap_or(DEFAULT_ICON_SIZE),
        ))
    })
    .await
}

/// 扩展名合法性：以 . 开头、总长 2..=16、仅含字母数字、点、下划线与连字符
fn validate_ext(ext: &str) -> Result<(), String> {
    if !ext.starts_with('.') || ext.len() < 2 || ext.len() > 16 {
        return Err("扩展名必须以 . 开头且长度不超过 16 字符".into());
    }
    if !ext[1..]
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err("扩展名包含非法字符".into());
    }
    Ok(())
}

/// 按扩展名提取系统文件图标（文件无需存在）：构造探测文件名直接调用
/// SHGetFileInfo，其对不存在的路径仍按扩展名关联返回图标，不触碰文件系统。
/// 用于右键菜单等按类型取图场景；失败返回 None（前端回退内置 SVG）。
fn icon_for_ext_impl(ext: &str) -> Result<Option<String>, String> {
    validate_ext(ext)?;
    icon_data_uri_for_ext(ext, DEFAULT_ICON_SIZE)
}

#[tauri::command]
pub async fn session_fs_icon_for_ext(ext: String) -> Result<Option<String>, String> {
    run_blocking(60, move || icon_for_ext_impl(&ext)).await
}

fn path_under_dot_dir(root: &Path, p: &Path) -> bool {
    shared_rel_path_of(root, p)
        .map(|rel| rel.split('/').any(is_noise_dir))
        .unwrap_or(false)
}

#[tauri::command]
pub async fn session_fs_watch_start(
    app: AppHandle,
    state: State<'_, FsWatcherState>,
    workspace: String,
) -> Result<(), String> {
    let root_p = resolve_workspace_dir(&workspace)?;
    let root_c = root_p
        .canonicalize()
        .map_err(|e| format!("无法解析工作目录 {}: {e}", workspace))?;

    {
        let guard = state.0.lock().map_err(|e| e.to_string())?;
        if let Some(h) = guard.as_ref() {
            if norm_key(&h.root) == norm_key(&root_c) {
                return Ok(()); // 已监听同一目录
            }
        }
    }

    // 停掉旧监听（换根或重复启动时）
    {
        let mut guard = state.0.lock().map_err(|e| e.to_string())?;
        guard.take();
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<PathBuf>(1024);
    let root_for_filter = root_c.clone();
    let tx_watcher = tx.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            for p in ev.paths {
                if path_under_dot_dir(&root_for_filter, &p) {
                    continue;
                }
                let _ = tx_watcher.try_send(p);
            }
        }
    })
    .map_err(|e| format!("创建文件监听失败: {e}"))?;

    watcher
        .watch(&root_c, RecursiveMode::Recursive)
        .map_err(|e| format!("监听目录失败 {}: {e}", workspace))?;

    // 防抖任务：300ms 静默后向前端 emit 变更事件
    let handle = app.clone();
    let root_for_task = root_c.clone();
    tauri::async_runtime::spawn(async move {
        let mut pending: Vec<String> = Vec::new();
        let mut last_event: Option<tokio::time::Instant> = None;
        let mut ticker = tokio::time::interval(Duration::from_millis(150));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(p) => {
                        pending.push(rel_path_of(&root_for_task, &p));
                        last_event = Some(tokio::time::Instant::now());
                    }
                    None => break,
                },
                _ = ticker.tick() => {
                    if let Some(t) = last_event {
                        if t.elapsed() >= Duration::from_millis(300) && !pending.is_empty() {
                            let payload = serde_json::json!({
                                "root": clean_path(&root_for_task),
                                "paths": pending,
                            });
                            let _ = handle.emit("session-fs/changed", payload);
                            pending.clear();
                            last_event = None;
                        }
                    }
                }
            }
        }
    });

    state
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .replace(WatchHandle {
            _watcher: watcher,
            _sender: tx,
            root: root_c,
        });
    Ok(())
}

#[tauri::command]
pub async fn session_fs_watch_stop(state: State<'_, FsWatcherState>) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    guard.take();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("b.txt"), "b").unwrap();
        std::fs::write(root.join("a.txt"), "a").unwrap();
        std::fs::write(root.join("src").join("main.ts"), "x").unwrap();
        std::fs::write(root.join(".gitignore"), "node_modules").unwrap();
        (tmp, root)
    }

    #[cfg(windows)]
    #[test]
    fn icons_impl_batch_and_fallback() {
        let (tmp, root) = tree();
        let req = vec![
            IconRequest {
                path: root.join("a.txt").to_string_lossy().into_owned(),
            },
            // 不存在/越界的路径：单条回退 None，不中断整批
            IconRequest {
                path: "C:\\Windows\\System32\\no-such-outside.exe".into(),
            },
        ];
        let out = icons_impl(&root, &req, 16);
        assert_eq!(out.len(), 2);
        assert!(out[0]
            .data_uri
            .as_deref()
            .unwrap_or("")
            .starts_with("data:image/png;base64,"));
        assert!(out[1].data_uri.is_none());
        let _ = tmp;
    }

    #[cfg(windows)]
    #[test]
    fn icon_for_ext_returns_txt_system_icon() {
        // 不存在的路径仍按扩展名返回系统 TXT 图标
        let uri = icon_for_ext_impl(".txt").unwrap();
        assert!(uri
            .as_deref()
            .unwrap_or("")
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn icon_for_ext_validates_input() {
        assert!(validate_ext(".txt").is_ok());
        assert!(validate_ext(".TXT").is_ok());
        assert!(validate_ext(".tar.gz").is_ok());
        assert!(validate_ext(".a_b-c").is_ok());
        // 无点/空/过短
        assert!(validate_ext("txt").is_err());
        assert!(validate_ext("").is_err());
        assert!(validate_ext(".").is_err());
        // 非法字符
        assert!(validate_ext(".tx t").is_err());
        assert!(validate_ext(".tx\t").is_err());
        assert!(validate_ext(".a/b").is_err());
        assert!(validate_ext(".a\\b").is_err());
        assert!(validate_ext(".\u{4e2d}").is_err());
        // 超长
        assert!(validate_ext(&format!(".{}", "a".repeat(20))).is_err());
        assert!(validate_ext(&format!(".{}", "a".repeat(15))).is_ok());
        // 非法输入直接 Err，不触碰取图标逻辑
        assert!(icon_for_ext_impl("txt").is_err());
        assert!(icon_for_ext_impl("").is_err());
    }

    #[test]
    fn validate_name_rules() {
        assert!(validate_name("a.txt").is_ok());
        assert!(validate_name("中文 文件").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name(" ").is_err());
        assert!(validate_name(".").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("a\\b").is_err());
        assert!(validate_name("a<b").is_err());
        assert!(validate_name("a?").is_err());
        assert!(validate_name("a.").is_err());
        assert!(validate_name(" a").is_err());
    }

    #[test]
    fn create_file_creates_empty_file_with_unique_name() {
        let (tmp, root) = tree();
        let dir = root.join("src");

        let e1 = create_file_impl(&root, &dir).unwrap();
        assert_eq!(e1.name, "新建文本文件.txt");
        assert_eq!(std::fs::read(dir.join("新建文本文件.txt")).unwrap(), b"");

        // 同名冲突 → (2)、(3)、(4) 递增
        let e2 = create_file_impl(&root, &dir).unwrap();
        assert_eq!(e2.name, "新建文本文件 (2).txt");
        let e3 = create_file_impl(&root, &dir).unwrap();
        assert_eq!(e3.name, "新建文本文件 (3).txt");
        let e4 = create_file_impl(&root, &dir).unwrap();
        assert_eq!(e4.name, "新建文本文件 (4).txt");

        // 条目指向目标目录内的真实文件
        assert!(dir.join(&e4.name).is_file());
        let _ = tmp;
    }

    #[test]
    fn create_file_rejects_non_dir_missing_and_outside() {
        let (tmp, root) = tree();

        // 目标是文件
        let err = create_file_impl(&root, &root.join("a.txt")).unwrap_err();
        assert!(err.contains("必须是目录"));

        // 目标不存在
        let err = create_file_impl(&root, &root.join("missing")).unwrap_err();
        assert!(err.contains("无法访问路径"));

        // 越出根目录
        let outside = tmp.path().parent().unwrap().to_path_buf();
        let err = create_file_impl(&root, &outside).unwrap_err();
        assert!(err.contains("路径越界"));
        let _ = tmp;
    }

    #[test]
    fn clean_path_strips_verbatim_prefix() {
        assert_eq!(clean_path(Path::new(r"\\?\C:\a\b.txt")), "C:\\a\\b.txt");
        assert_eq!(
            clean_path(Path::new(r"\\?\UNC\srv\share\f.txt")),
            "\\\\srv\\share\\f.txt"
        );
        assert_eq!(clean_path(Path::new(r"D:\plain\path")), "D:\\plain\\path");
        // 对外绝对路径统一反斜杠（git 等外部输出正斜杠时也归一）
        assert_eq!(clean_path(Path::new("C:/mixed/path")), "C:\\mixed\\path");
    }

    #[test]
    fn entries_expose_clean_paths() {
        let (_tmp, root) = tree();
        for e in list_impl(&root, &root).unwrap() {
            assert!(!e.path.starts_with(r"\\?\"));
        }
        let meta = metadata_impl(&root, &root.join("src").join("main.ts")).unwrap();
        assert!(!meta.path.starts_with(r"\\?\"));
        assert_eq!(meta.rel_path, "src/main.ts");
    }

    #[cfg(windows)]
    #[test]
    fn root_display_name_uses_clean_path() {
        // 盘符根目录没有 file_name，回退必须去掉 canonicalize 的 \\?\ 前缀
        assert_eq!(display_name(Path::new(r"\\?\C:\")), "C:\\");
        assert_eq!(display_name(Path::new(r"\\?\C:\codex\ui")), "ui");
        // UNC 根同样没有 file_name，回退结果必须与 path 字段（clean_path）一致
        let unc = Path::new(r"\\?\UNC\srv\share\");
        assert_eq!(display_name(unc), clean_path(unc));
        assert_eq!(display_name(Path::new(r"\\?\UNC\srv\share\f.txt")), "f.txt");
    }

    #[test]
    fn ensure_inside_boundary() {
        let (tmp, root) = tree();
        let inside = root.join("src").join("main.ts");
        assert!(ensure_inside(&root, &inside).is_ok());
        assert!(ensure_inside(&root, &root).is_ok());
        let outside = tmp.path().parent().unwrap().join("outside-x.txt");
        std::fs::write(&outside, "x").unwrap();
        assert!(ensure_inside(&root, &outside).is_err());
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn ensure_inside_resolves_relative_path_against_root() {
        let (_tmp, root) = tree();
        // 相对路径按 root 拼接（工作区外文件以父目录为根、传文件名的场景）
        let resolved = ensure_inside(&root, Path::new("src/main.ts")).unwrap();
        assert_eq!(
            resolved,
            root.join("src").join("main.ts").canonicalize().unwrap()
        );
        // 相对路径指向 root 下不存在的文件：仍报错
        assert!(ensure_inside(&root, Path::new("missing.txt")).is_err());
        // 相对路径逃逸 root（..）：拼接规范化后被越界校验拒绝
        let escape = root.join("..").join("escape-x.txt");
        std::fs::write(&escape, "x").unwrap();
        assert!(ensure_inside(&root, Path::new("../escape-x.txt")).is_err());
        let _ = std::fs::remove_file(&escape);
    }

    #[test]
    fn list_filters_dot_dirs_and_sorts() {
        let (_tmp, root) = tree();
        let entries = list_impl(&root, &root).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        // 目录优先、A-Z；.git 与 node_modules 被过滤；点文件 .gitignore 保留
        assert_eq!(names, vec!["src", ".gitignore", "a.txt", "b.txt"]);
        let src = entries.iter().find(|e| e.name == "src").unwrap();
        assert!(src.is_dir);
        assert_eq!(src.child_count, Some(1));
    }

    #[test]
    fn search_matches_name_ignores_dot_dirs() {
        let (_tmp, root) = tree();
        let hits = search_impl(&root, "MAIN", 100).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "main.ts");
        assert_eq!(hits[0].rel_path, "src/main.ts");
        // 点目录内容不参与搜索
        std::fs::write(root.join(".git").join("secret.txt"), "x").unwrap();
        let hits = search_impl(&root, "secret", 100).unwrap();
        assert!(hits.is_empty());
        // node_modules 无条件忽略：目录本身与其内容均不参与搜索
        std::fs::write(root.join("node_modules").join("secret.js"), "x").unwrap();
        let hits = search_impl(&root, "secret", 100).unwrap();
        assert!(hits.is_empty());
        let hits = search_impl(&root, "node_modules", 100).unwrap();
        assert!(hits.is_empty());
        // 空查询与上限
        assert!(search_impl(&root, "  ", 100).unwrap().is_empty());
        assert!(search_impl(&root, "a", 1).unwrap().len() <= 1);
    }

    #[test]
    fn rg_exe_in_requires_regular_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(rg_exe_in(tmp.path()).is_none());
        std::fs::create_dir_all(tmp.path().join("bin").join("rg.exe")).unwrap();
        assert!(rg_exe_in(tmp.path()).is_none());
        std::fs::remove_dir_all(tmp.path().join("bin").join("rg.exe")).unwrap();
        std::fs::write(tmp.path().join("bin").join("rg.exe"), b"MZ").unwrap();
        assert_eq!(
            rg_exe_in(tmp.path()),
            Some(tmp.path().join("bin").join("rg.exe"))
        );
    }

    #[test]
    fn rg_args_lock_search_semantics() {
        let args = rg_args("needle", Path::new("C:\\repo"));
        for expected in [
            "--json",
            "--fixed-strings",
            "--ignore-case",
            "--max-count",
            "1",
            "--max-filesize",
            "2M",
            "--no-messages",
            "--glob",
            "!**/node_modules/**",
            "--",
            "needle",
            "C:\\repo",
        ] {
            assert!(args.iter().any(|a| a == expected), "missing {expected}");
        }
        assert_eq!(args[args.len() - 2], "needle");
    }

    #[test]
    fn parse_rg_json_extracts_first_match_and_skips_invalid() {
        let (tmp, root) = tree();
        std::fs::write(root.join("a.txt"), "needle\n").unwrap();
        std::fs::write(root.join("src").join("main.ts"), "const needle = 1;\n").unwrap();
        let outside = tmp.path().parent().unwrap().join("outside-rg.txt");
        std::fs::write(&outside, "needle").unwrap();
        let a_path = root.join("a.txt").to_string_lossy().into_owned();
        let main_path = root
            .join("src")
            .join("main.ts")
            .to_string_lossy()
            .into_owned();
        let outside_path = outside.to_string_lossy().into_owned();
        let lines = [
            serde_json::json!({
                "type": "match",
                "data": {
                    "path": { "text": a_path.as_str() },
                    "line_number": 2,
                    "lines": { "text": "  needle here  \n" }
                }
            })
            .to_string(),
            "not-json".to_string(),
            serde_json::json!({
                "type": "match",
                "data": {
                    "path": { "text": a_path.as_str() },
                    "line_number": 3,
                    "lines": { "text": "duplicate" }
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "match",
                "data": {
                    "path": { "text": outside_path.as_str() },
                    "line_number": 1,
                    "lines": { "text": "outside" }
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "match",
                "data": {
                    "path": { "text": main_path.as_str() },
                    "line_number": 4,
                    "lines": { "text": "const needle = 1;\n" }
                }
            })
            .to_string(),
        ]
        .join("\n");
        let hits = parse_rg_json(&lines, &root, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].entry.rel_path, "a.txt");
        assert_eq!(hits[0].line_number, 2);
        assert_eq!(hits[0].line_text, "needle here");
        assert_eq!(hits[1].entry.rel_path, "src/main.ts");
        assert_eq!(hits[1].line_number, 4);
        let _ = std::fs::remove_file(outside);
        let _ = tmp;
    }

    #[test]
    fn parse_rg_json_truncates_and_respects_limit() {
        let (_tmp, root) = tree();
        let long = "x".repeat(250);
        let a_path = root.join("a.txt").to_string_lossy().into_owned();
        let line = serde_json::json!({
            "type": "match",
            "data": {
                "path": { "text": a_path },
                "line_number": 1,
                "lines": { "text": long }
            }
        })
        .to_string();
        let hits = parse_rg_json(&line, &root, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line_text.chars().count(), 201);
        assert!(hits[0].line_text.ends_with('…'));
        assert!(parse_rg_json(&line, &root, 0).is_empty());
    }

    #[test]
    fn search_rg_impl_missing_binary_is_unavailable() {
        let (_tmp, root) = tree();
        let result = search_rg_impl(&root, "x", 10, None);
        assert!(!result.available);
        assert!(result.hits.is_empty());
    }

    #[test]
    fn watcher_filter_excludes_dot_dirs_and_node_modules() {
        let (_tmp, root) = tree();
        std::fs::create_dir_all(root.join("node_modules").join("pkg")).unwrap();
        std::fs::create_dir_all(root.join("src").join("nested")).unwrap();
        assert!(path_under_dot_dir(&root, &root.join(".git").join("index")));
        assert!(path_under_dot_dir(&root, &root.join(".vs").join("x")));
        assert!(path_under_dot_dir(
            &root,
            &root.join("node_modules").join("pkg").join("index.js")
        ));
        assert!(path_under_dot_dir(
            &root,
            &root
                .join("src")
                .join("nested")
                .join("node_modules")
                .join("a.js")
        ));
        assert!(!path_under_dot_dir(
            &root,
            &root.join("src").join("nested").join("a.js")
        ));
        assert!(!path_under_dot_dir(&root, &root.join("a.txt")));
    }

    #[test]
    fn rename_rejects_root_and_bad_names() {
        let (_tmp, root) = tree();
        assert!(rename_impl(&root, &root, "new").is_err());
        assert!(rename_impl(&root, &root.join("a.txt"), "a/b").is_err());
        let e = rename_impl(&root, &root.join("a.txt"), "a2.txt").unwrap();
        assert_eq!(e.name, "a2.txt");
        assert!(rename_impl(&root, &root.join("a2.txt"), "b.txt").is_err()); // 已存在
    }

    #[test]
    fn delete_removes_file_and_dir_recursively() {
        let (_tmp, root) = tree();
        assert!(delete_impl(&root, &root).is_err());
        delete_impl(&root, &root.join("src")).unwrap();
        assert!(!root.join("src").exists());
        delete_impl(&root, &root.join("a.txt")).unwrap();
        assert!(!root.join("a.txt").exists());
    }

    #[test]
    fn copy_and_paste_recursive_with_guards() {
        let (_tmp, root) = tree();
        // 复制目录到另一目录
        let e = copy_impl(&root, &root.join("src"), &root.join("node_modules")).unwrap();
        assert!(e.is_dir);
        assert!(root
            .join("node_modules")
            .join("src")
            .join("main.ts")
            .exists());
        // 防止复制到自身
        assert!(copy_impl(&root, &root.join("src"), &root.join("src")).is_err());
        // paste 支持外部源（此处用同一 root 内文件模拟）
        let created = paste_impl(
            &root,
            &root.join("src"),
            &[root.join("b.txt").to_string_lossy().into_owned()],
        )
        .unwrap();
        assert_eq!(created.len(), 1);
        assert!(root.join("src").join("b.txt").exists());
        // 防止粘贴到自身
        assert!(paste_impl(
            &root,
            &root.join("src"),
            &[root.join("src").to_string_lossy().into_owned()]
        )
        .is_err());
    }

    #[test]
    fn create_dir_creates_folder_with_unique_name() {
        let (tmp, root) = tree();
        let dir = root.join("src");

        let e1 = create_dir_impl(&root, &dir).unwrap();
        assert_eq!(e1.name, "新建文件夹");
        assert!(dir.join("新建文件夹").is_dir());

        // 同名冲突 → (2)、(3) 递增
        let e2 = create_dir_impl(&root, &dir).unwrap();
        assert_eq!(e2.name, "新建文件夹 (2)");
        let e3 = create_dir_impl(&root, &dir).unwrap();
        assert_eq!(e3.name, "新建文件夹 (3)");

        // 条目指向目标目录内的真实目录
        assert!(dir.join(&e3.name).is_dir());
        let _ = tmp;
    }

    #[test]
    fn create_dir_rejects_non_dir_missing_and_outside() {
        let (tmp, root) = tree();

        // 目标是文件
        let err = create_dir_impl(&root, &root.join("a.txt")).unwrap_err();
        assert!(err.contains("必须是目录"));

        // 目标不存在
        let err = create_dir_impl(&root, &root.join("missing")).unwrap_err();
        assert!(err.contains("无法访问路径"));

        // 越出根目录
        let outside = tmp.path().parent().unwrap().to_path_buf();
        let err = create_dir_impl(&root, &outside).unwrap_err();
        assert!(err.contains("路径越界"));
        let _ = tmp;
    }

    #[test]
    fn move_moves_file_and_dir_with_guards() {
        let (_tmp, root) = tree();

        // 文件移动到子目录：rename 成功且源消失
        let e = move_impl(&root, &root.join("a.txt"), &root.join("src")).unwrap();
        assert_eq!(e.name, "a.txt");
        assert!(root.join("src").join("a.txt").exists());
        assert!(!root.join("a.txt").exists());

        // 目录移动到另一目录（递归）
        let e = move_impl(&root, &root.join("src"), &root.join("node_modules")).unwrap();
        assert!(e.is_dir);
        assert!(root
            .join("node_modules")
            .join("src")
            .join("main.ts")
            .exists());
        assert!(!root.join("src").exists());

        // 移动到自身所在目录（目标已存在）
        assert!(move_impl(&root, &root.join("b.txt"), &root).is_err());

        // 目录移动到自身
        std::fs::create_dir_all(root.join("sub")).unwrap();
        assert!(move_impl(&root, &root.join("sub"), &root.join("sub")).is_err());

        // 移动到自身子目录
        std::fs::create_dir_all(root.join("sub").join("inner")).unwrap();
        let err = move_impl(&root, &root.join("sub"), &root.join("sub").join("inner")).unwrap_err();
        assert!(err.contains("子目录"));

        // 移动根被拒
        assert!(move_impl(&root, &root, &root.join("sub")).is_err());

        // 目标已存在
        std::fs::write(root.join("sub").join(".gitignore"), "x").unwrap();
        let err = move_impl(&root, &root.join(".gitignore"), &root.join("sub")).unwrap_err();
        assert!(err.contains("目标已存在"));
    }

    #[test]
    fn move_rejects_missing_and_outside() {
        let (tmp, root) = tree();

        // 源不存在
        let err = move_impl(&root, &root.join("missing.txt"), &root.join("src")).unwrap_err();
        assert!(err.contains("无法访问路径"));

        // 目标必须是目录
        let err = move_impl(&root, &root.join("a.txt"), &root.join("a.txt")).unwrap_err();
        assert!(err.contains("目标必须是目录"));

        // 越出根目录
        let outside = tmp.path().parent().unwrap().to_path_buf();
        let err = move_impl(&root, &root.join("a.txt"), &outside).unwrap_err();
        assert!(err.contains("路径越界"));
        let _ = tmp;
    }

    #[test]
    fn read_file_with_guards() {
        let (tmp, root) = tree();
        // 正常读取：内容原样返回 + UTF-8 标记与字节数
        let c = read_impl(&root, &root.join("a.txt")).unwrap();
        assert_eq!(c.content, "a");
        assert!(c.valid_utf8);
        assert_eq!(c.byte_size, 1);
        let c = read_impl(&root, &root.join("src").join("main.ts")).unwrap();
        assert_eq!(c.content, "x");
        assert!(c.valid_utf8);
        // 目录拒绝
        assert!(read_impl(&root, &root.join("src")).is_err());
        // 越界拒绝（ensure_inside 先行拦截）
        let outside = tmp.path().parent().unwrap().join("outside-read.txt");
        std::fs::write(&outside, "x").unwrap();
        assert!(read_impl(&root, &outside).is_err());
        let _ = std::fs::remove_file(&outside);
        // 超过 5 MiB 拒绝
        std::fs::write(root.join("big.bin"), vec![0u8; (5 * 1024 * 1024) + 1]).unwrap();
        assert!(read_impl(&root, &root.join("big.bin")).is_err());
    }

    #[test]
    fn read_invalid_utf8_marks_readonly() {
        let (_tmp, root) = tree();
        std::fs::write(root.join("bad.dat"), [0xffu8, 0xfeu8]).unwrap();
        let c = read_impl(&root, &root.join("bad.dat")).unwrap();
        assert!(!c.valid_utf8);
        assert_eq!(c.byte_size, 2);
    }

    #[test]
    fn write_file_roundtrip_with_guards() {
        let (tmp, root) = tree();
        // 正常写入：UTF-8 多字节 + 返回刷新后的条目
        let e = write_impl(&root, &root.join("a.txt"), "你好，Codex！\nline2").unwrap();
        assert_eq!(e.name, "a.txt");
        let c = read_impl(&root, &root.join("a.txt")).unwrap();
        assert_eq!(c.content, "你好，Codex！\nline2");
        assert!(c.valid_utf8);
        // 目录拒绝
        assert!(write_impl(&root, &root.join("src"), "x").is_err());
        // 越界拒绝
        let outside = tmp.path().parent().unwrap().join("outside-write.txt");
        std::fs::write(&outside, "x").unwrap();
        assert!(write_impl(&root, &outside, "y").is_err());
        let _ = std::fs::remove_file(&outside);
        // 不存在的文件拒绝（ensure_inside canonicalize 失败）
        assert!(write_impl(&root, &root.join("missing.txt"), "y").is_err());
        // 含 NUL 拒绝
        assert!(write_impl(&root, &root.join("a.txt"), "a\u{0}b").is_err());
        // 超过 5 MiB 拒绝
        let big = "x".repeat((5 * 1024 * 1024) + 1);
        assert!(write_impl(&root, &root.join("a.txt"), &big).is_err());
    }

    #[test]
    fn probe_text_detects_content() {
        let (tmp, root) = tree();
        // 文本：ASCII
        assert!(probe_text_impl(&root, &root.join("a.txt")).unwrap());
        // 文本：UTF-8 中文
        std::fs::write(root.join("zh.txt"), "你好，Codex！").unwrap();
        assert!(probe_text_impl(&root, &root.join("zh.txt")).unwrap());
        // 空文件
        std::fs::write(root.join("empty.txt"), "").unwrap();
        assert!(probe_text_impl(&root, &root.join("empty.txt")).unwrap());
        // 超过采样窗口的 UTF-8 文本：采样边界截断多字节序列不误判为二进制
        std::fs::write(root.join("long.txt"), "中".repeat(4000)).unwrap();
        assert!(probe_text_impl(&root, &root.join("long.txt")).unwrap());
        // 含 NUL 的二进制
        std::fs::write(root.join("bin.dat"), [0u8; 100]).unwrap();
        assert!(!probe_text_impl(&root, &root.join("bin.dat")).unwrap());
        // 无 NUL 但非法 UTF-8
        std::fs::write(root.join("bad.dat"), [0xffu8, 0xfeu8]).unwrap();
        assert!(!probe_text_impl(&root, &root.join("bad.dat")).unwrap());
        // NUL 位于采样窗口之外时不误判（记录窗口限制）
        let mut late = vec![b'a'; 8100];
        late.push(0);
        std::fs::write(root.join("late-nul.dat"), late).unwrap();
        assert!(probe_text_impl(&root, &root.join("late-nul.dat")).unwrap());
        // 目录拒绝
        assert!(probe_text_impl(&root, &root.join("src")).is_err());
        // 越界拒绝
        let outside = tmp.path().parent().unwrap().join("outside-probe.dat");
        std::fs::write(&outside, "x").unwrap();
        assert!(probe_text_impl(&root, &outside).is_err());
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn read_bytes_roundtrip_with_guards() {
        let (tmp, root) = tree();
        // 正常读取：原始字节直返
        let payload = [0x25u8, 0x50, 0x44, 0x46, 0x2d, 0x31]; // %PDF-1
        std::fs::write(root.join("doc.pdf"), payload).unwrap();
        let c = read_bytes_impl(&root, &root.join("doc.pdf"), 1024).unwrap();
        assert_eq!(c, payload);
        // 目录拒绝
        assert!(read_bytes_impl(&root, &root.join("src"), 1024).is_err());
        // 越界拒绝
        let outside = tmp.path().parent().unwrap().join("outside.pdf");
        std::fs::write(&outside, b"x").unwrap();
        assert!(read_bytes_impl(&root, &outside, 1024).is_err());
        let _ = std::fs::remove_file(&outside);
        // 超过上限拒绝
        std::fs::write(root.join("big.pdf"), vec![0u8; 2048]).unwrap();
        let err = read_bytes_impl(&root, &root.join("big.pdf"), 1024).unwrap_err();
        assert!(err.contains("文件过大"));
    }

    #[test]
    fn write_bytes_roundtrip_with_guards() {
        let (tmp, root) = tree();
        // 正常写入：base64 解码后覆盖原文件
        let payload = [0x50u8, 0x4b, 0x03, 0x04, 0x14, 0x00]; // PK\x03\x04
        std::fs::write(root.join("doc.docx"), b"old").unwrap();
        let e = write_bytes_impl(
            &root,
            &root.join("doc.docx"),
            &STANDARD.encode(payload),
            1024,
        )
        .unwrap();
        assert_eq!(e.name, "doc.docx");
        assert_eq!(std::fs::read(root.join("doc.docx")).unwrap(), payload);
        // 目录拒绝
        assert!(
            write_bytes_impl(&root, &root.join("src"), &STANDARD.encode(payload), 1024).is_err()
        );
        // 越界拒绝
        let outside = tmp.path().parent().unwrap().join("outside.docx");
        std::fs::write(&outside, b"x").unwrap();
        assert!(write_bytes_impl(&root, &outside, &STANDARD.encode(payload), 1024).is_err());
        let _ = std::fs::remove_file(&outside);
        // 非法 base64 拒绝
        let err =
            write_bytes_impl(&root, &root.join("doc.docx"), "not-base64!!!", 1024).unwrap_err();
        assert!(err.contains("base64"));
        // 超过上限拒绝
        let big = vec![0u8; 2048];
        let err = write_bytes_impl(&root, &root.join("doc.docx"), &STANDARD.encode(&big), 1024)
            .unwrap_err();
        assert!(err.contains("文件过大"));
        // 超长文本在解码前即拒绝（不分配大内存）
        let huge = "A".repeat(4096);
        let err = write_bytes_impl(&root, &root.join("doc.docx"), &huge, 1024).unwrap_err();
        assert!(err.contains("文件过大"));
        let _ = tmp;
    }
}
