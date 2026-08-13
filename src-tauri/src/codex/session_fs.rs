use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::codex::path_util::{clean_path, is_inside_path, norm_key};

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

/// 规范化路径键：统一反斜杠、去尾部分隔符、小写（Windows 大小写不敏感）
/// 相对 root 的相对路径（. 表示根）；对规范化/原始路径混用做大小写不敏感匹配，
/// 输出保留原始大小写、统一正斜杠分隔。
fn rel_path_of(root: &Path, path: &Path) -> String {
    let root_s = clean_path(root).replace('/', "\\");
    let path_s = clean_path(path).replace('/', "\\");
    let root_l = root_s.to_lowercase();
    let path_l = path_s.to_lowercase();
    if path_l == root_l {
        return ".".to_string();
    }
    if let Some(rest) = path_l.strip_prefix(&root_l) {
        if rest.starts_with('\\') {
            let orig_rest = &path_s[root_l.len() + 1..];
            return orig_rest.replace('\\', "/");
        }
    }
    path_s.replace('\\', "/")
}

/// target 必须位于 root 内（规范化后，Windows 大小写不敏感）
fn ensure_inside(root: &Path, target: &Path) -> Result<PathBuf, String> {
    let root_c = root
        .canonicalize()
        .map_err(|e| format!("无法访问工作目录 {}: {e}", clean_path(root)))?;
    let target_c = target
        .canonicalize()
        .map_err(|e| format!("无法访问路径 {}: {e}", clean_path(target)))?;
    if is_inside_path(&root_c, &target_c) {
        Ok(target_c)
    } else {
        Err(format!("路径越界: {}", clean_path(target)))
    }
}

fn resolve_root(root: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(root);
    if !p.is_absolute() {
        return Err("工作目录必须为绝对路径".into());
    }
    let meta = std::fs::metadata(&p).map_err(|e| format!("无法访问工作目录 {root}: {e}"))?;
    if !meta.is_dir() {
        return Err(format!("工作目录不是目录: {root}"));
    }
    Ok(p)
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

fn entry_from_path(root: &Path, path: &Path) -> Result<FsEntry, String> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("读取元信息失败 {}: {e}", clean_path(path)))?;
    let is_dir = meta.file_type().is_dir();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let rel_path = rel_path_of(root, path);
    Ok(FsEntry {
        name,
        path: clean_path(path),
        rel_path,
        is_dir,
        size: if is_dir { None } else { Some(meta.len()) },
        modified_at_ms: ts_ms(meta.modified().ok()),
        created_at_ms: ts_ms(meta.created().ok()),
        child_count: if is_dir { Some(visible_child_count(path)) } else { None },
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
    let rd = std::fs::read_dir(&dir_c)
        .map_err(|e| format!("读取目录失败 {}: {e}", clean_path(dir)))?;
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
        std::fs::remove_dir_all(&c)
            .map_err(|e| format!("删除目录失败 {}: {e}", clean_path(&c)))?;
    } else {
        std::fs::remove_file(&c)
            .map_err(|e| format!("删除文件失败 {}: {e}", clean_path(&c)))?;
    }
    Ok(())
}

fn copy_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(src)
        .map_err(|e| format!("读取元信息失败 {}: {e}", clean_path(src)))?;
    if meta.file_type().is_dir() {
        std::fs::create_dir_all(dst)
            .map_err(|e| format!("创建目录失败 {}: {e}", clean_path(dst)))?;
        let rd = std::fs::read_dir(src)
            .map_err(|e| format!("读取目录失败 {}: {e}", clean_path(src)))?;
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
            return Err(format!("不能把「{}」粘贴到自身或子目录", clean_path(&src_c)));
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

#[tauri::command]
pub async fn session_fs_list(root: String, dir: String) -> Result<Vec<FsEntry>, String> {
    run_blocking(60, move || {
        let root_p = resolve_root(&root)?;
        list_impl(&root_p, Path::new(&dir))
    })
    .await
}

#[tauri::command]
pub async fn session_fs_search(
    root: String,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<FsEntry>, String> {
    run_blocking(120, move || {
        let root_p = resolve_root(&root)?;
        search_impl(&root_p, &query, limit.unwrap_or(200))
    })
    .await
}

#[tauri::command]
pub async fn session_fs_metadata(root: String, path: String) -> Result<FsEntry, String> {
    run_blocking(60, move || {
        let root_p = resolve_root(&root)?;
        metadata_impl(&root_p, Path::new(&path))
    })
    .await
}

#[tauri::command]
pub async fn session_fs_rename(
    root: String,
    path: String,
    new_name: String,
) -> Result<FsEntry, String> {
    run_blocking(60, move || {
        let root_p = resolve_root(&root)?;
        rename_impl(&root_p, Path::new(&path), &new_name)
    })
    .await
}

#[tauri::command]
pub async fn session_fs_delete(root: String, path: String) -> Result<(), String> {
    run_blocking(60, move || {
        let root_p = resolve_root(&root)?;
        delete_impl(&root_p, Path::new(&path))
    })
    .await
}

#[tauri::command]
pub async fn session_fs_copy(
    root: String,
    src: String,
    dest_dir: String,
) -> Result<FsEntry, String> {
    run_blocking(60, move || {
        let root_p = resolve_root(&root)?;
        copy_impl(&root_p, Path::new(&src), Path::new(&dest_dir))
    })
    .await
}

#[tauri::command]
pub async fn session_fs_paste(
    root: String,
    dest_dir: String,
    sources: Vec<String>,
) -> Result<Vec<FsEntry>, String> {
    run_blocking(60, move || {
        let root_p = resolve_root(&root)?;
        paste_impl(&root_p, Path::new(&dest_dir), &sources)
    })
    .await
}

/// 文本预览最大读取字节数（1 MiB）
const MAX_PREVIEW_BYTES: u64 = 1024 * 1024;

/// 单文件内容读取（文本预览用）：路径包含校验、仅文件、大小上限、UTF-8 lossy
fn read_impl(root: &Path, path: &Path) -> Result<String, String> {
    let target = ensure_inside(root, path)?;
    let meta = std::fs::metadata(&target)
        .map_err(|e| format!("无法读取文件 {}: {e}", clean_path(&target)))?;
    if !meta.is_file() {
        return Err(format!("不是文件: {}", clean_path(&target)));
    }
    if meta.len() > MAX_PREVIEW_BYTES {
        return Err("文件过大，暂不支持预览".into());
    }
    let bytes = std::fs::read(&target)
        .map_err(|e| format!("无法读取文件 {}: {e}", clean_path(&target)))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 在阻塞线程中执行同步文件操作，带超时（秒）
async fn run_blocking<T: Send + 'static>(
    timeout_secs: u64,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let task = tokio::task::spawn_blocking(f);
    match tokio::time::timeout(Duration::from_secs(timeout_secs), task).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => Err(format!("文件操作任务异常: {e}")),
        Err(_) => Err(format!("文件操作超时（{timeout_secs} 秒）")),
    }
}

#[tauri::command]
pub async fn session_fs_read(root: String, path: String) -> Result<String, String> {
    run_blocking(60, move || {
        let root_p = resolve_root(&root)?;
        read_impl(&root_p, Path::new(&path))
    })
    .await
}

fn path_under_dot_dir(root: &Path, p: &Path) -> bool {
    p.strip_prefix(root)
        .map(|rel| {
            rel.components().any(|c| {
                matches!(c, Component::Normal(n) if {
                    let s = n.to_string_lossy();
                    is_noise_dir(&s)
                })
            })
        })
        .unwrap_or(false)
}

fn path_to_rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .map(|r| r.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| p.to_string_lossy().replace('\\', "/"))
}

#[tauri::command]
pub async fn session_fs_watch_start(
    app: AppHandle,
    state: State<'_, FsWatcherState>,
    root: String,
) -> Result<(), String> {
    let root_p = resolve_root(&root)?;
    let root_c = root_p
        .canonicalize()
        .map_err(|e| format!("无法解析工作目录 {}: {e}", root))?;

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
        .map_err(|e| format!("监听目录失败 {}: {e}", root))?;

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
                        pending.push(path_to_rel(&root_for_task, &p));
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

    state.0.lock().map_err(|e| e.to_string())?.replace(WatchHandle {
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
    fn clean_path_strips_verbatim_prefix() {
        assert_eq!(clean_path(Path::new(r"\\?\C:\a\b.txt")), "C:\\a\\b.txt");
        assert_eq!(
            clean_path(Path::new(r"\\?\UNC\srv\share\f.txt")),
            "\\\\srv\\share\\f.txt"
        );
        assert_eq!(clean_path(Path::new(r"D:\plain\path")), "D:\\plain\\path");
        assert_eq!(clean_path(Path::new("C:/mixed/path")), "C:/mixed/path");
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
    fn watcher_filter_excludes_dot_dirs_and_node_modules() {
        let (_tmp, root) = tree();
        std::fs::create_dir_all(root.join("node_modules").join("pkg")).unwrap();
        std::fs::create_dir_all(root.join("src").join("nested")).unwrap();
        assert!(path_under_dot_dir(&root, &root.join(".git").join("index")));
        assert!(path_under_dot_dir(&root, &root.join(".vs").join("x")));
        assert!(path_under_dot_dir(&root, &root.join("node_modules").join("pkg").join("index.js")));
        assert!(path_under_dot_dir(&root, &root.join("src").join("nested").join("node_modules").join("a.js")));
        assert!(!path_under_dot_dir(&root, &root.join("src").join("nested").join("a.js")));
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
        assert!(root.join("node_modules").join("src").join("main.ts").exists());
        // 防止复制到自身
        assert!(copy_impl(&root, &root.join("src"), &root.join("src")).is_err());
        // paste 支持外部源（此处用同一 root 内文件模拟）
        let created = paste_impl(&root, &root.join("src"), &[root.join("b.txt").to_string_lossy().into_owned()])
            .unwrap();
        assert_eq!(created.len(), 1);
        assert!(root.join("src").join("b.txt").exists());
        // 防止粘贴到自身
        assert!(paste_impl(&root, &root.join("src"), &[root.join("src").to_string_lossy().into_owned()]).is_err());
    }

    #[test]
    fn read_file_with_guards() {
        let (tmp, root) = tree();
        // 正常读取：内容原样返回
        assert_eq!(read_impl(&root, &root.join("a.txt")).unwrap(), "a");
        assert_eq!(
            read_impl(&root, &root.join("src").join("main.ts")).unwrap(),
            "x"
        );
        // 目录拒绝
        assert!(read_impl(&root, &root.join("src")).is_err());
        // 越界拒绝（ensure_inside 先行拦截）
        let outside = tmp.path().parent().unwrap().join("outside-read.txt");
        std::fs::write(&outside, "x").unwrap();
        assert!(read_impl(&root, &outside).is_err());
        let _ = std::fs::remove_file(&outside);
        // 超过 1 MiB 拒绝
        std::fs::write(root.join("big.bin"), vec![0u8; (1024 * 1024) + 1]).unwrap();
        assert!(read_impl(&root, &root.join("big.bin")).is_err());
    }
}
