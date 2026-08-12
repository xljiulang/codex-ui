use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

/// git 错误码：前端据此渲染不同的空状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GitErrorCode {
    /// 系统中找不到 git 可执行文件
    GitNotFound,
    /// 目录不在任何 Git 仓库内
    NotARepo,
    /// 其它仓库/命令错误
    RepoError,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitError {
    pub code: GitErrorCode,
    pub message: String,
}

fn git_err(msg: impl Into<String>) -> GitError {
    GitError {
        code: GitErrorCode::RepoError,
        message: msg.into(),
    }
}

/// 变更文件条目
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFile {
    /// 相对仓库根的路径（正斜杠分隔）
    pub path: String,
    /// added | modified | deleted | renamed | untracked | conflicted
    pub status: String,
}

/// Git 状态（供前端展示）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    /// 仓库根目录（Windows 反斜杠路径）
    pub repo_root: String,
    /// 当前分支名；游离 HEAD 时为 "HEAD"
    pub branch: String,
    pub files: Vec<GitFile>,
}

/// 文件监听句柄（sender 被 drop 时防抖任务随 rx 关闭退出）
pub struct GitWatchHandle {
    _watcher: RecommendedWatcher,
    _sender: tokio::sync::mpsc::Sender<PathBuf>,
    root: PathBuf,
}

pub struct GitWatcherState(pub Mutex<Option<GitWatchHandle>>);

// ---------- git 命令执行 ----------

fn args(s: &[&str]) -> Vec<String> {
    s.iter().map(|x| x.to_string()).collect()
}

/// 同步执行 git，返回 (退出码, stdout, stderr)；git 不存在 → GitNotFound
fn run_git(args: &[String]) -> Result<(i32, String, String), GitError> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 避免 GUI 程序每次拉起 git 时弹出控制台黑窗体
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            GitError {
                code: GitErrorCode::GitNotFound,
                message: "未检测到 Git：请先安装 Git 并确保其位于 PATH 中".to_string(),
            }
        } else {
            git_err(format!("无法执行 git: {e}"))
        }
    })?;
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Ok((code, stdout, stderr))
}

/// 解析仓库根目录（失败时区分 not_a_repo / 其它错误）
fn resolve_repo_root_sync(path: &str) -> Result<String, GitError> {
    let (code, stdout, stderr) = run_git(&args(&["-C", path, "rev-parse", "--show-toplevel"]))?;
    if code == 0 {
        return Ok(stdout.trim().replace('/', "\\"));
    }
    if stderr.contains("not a git repository") {
        return Err(GitError {
            code: GitErrorCode::NotARepo,
            message: "当前目录不在任何 Git 仓库内".to_string(),
        });
    }
    Err(git_err(if stderr.trim().is_empty() {
        format!("无法识别 Git 仓库（退出码 {code}）")
    } else {
        stderr.trim().to_string()
    }))
}

/// 由 porcelain v2 的 XY 字段映射展示状态
fn xy_status(xy: &str) -> String {
    let b: Vec<char> = xy.chars().collect();
    let (a, c) = if b.len() >= 2 { (b[0], b[1]) } else { ('.', '.') };
    if a == 'D' || c == 'D' {
        "deleted".into()
    } else if a == 'A' || c == 'A' {
        "added".into()
    } else {
        "modified".into()
    }
}

/// 解析 `git status --porcelain=v2 -z` 输出（记录按 NUL 分隔，路径可能含空格）
fn parse_porcelain_v2(out: &str) -> Vec<GitFile> {
    let records: Vec<&str> = out.split('\0').collect();
    let mut files: Vec<GitFile> = Vec::new();
    let mut i = 0;
    while i < records.len() {
        let rec = records[i];
        i += 1;
        if rec.is_empty() {
            continue;
        }
        if let Some(rest) = rec.strip_prefix("2 ") {
            // 重命名：本记录含新路径，下一条裸记录是原路径
            let mut parts = rest.splitn(9, ' ');
            let _ = parts.next(); // XY
            for _ in 0..7 {
                let _ = parts.next();
            }
            let path = parts.next().unwrap_or("").to_string();
            files.push(GitFile {
                path,
                status: "renamed".into(),
            });
            if i < records.len() {
                i += 1; // 跳过原路径记录
            }
            continue;
        }
        if let Some(rest) = rec.strip_prefix("1 ") {
            let mut parts = rest.splitn(8, ' ');
            let xy = parts.next().unwrap_or("");
            for _ in 0..6 {
                let _ = parts.next();
            }
            let path = parts.next().unwrap_or("").to_string();
            files.push(GitFile {
                path,
                status: xy_status(xy),
            });
            continue;
        }
        if let Some(rest) = rec.strip_prefix("u ") {
            let mut parts = rest.splitn(10, ' ');
            let _ = parts.next(); // XY
            for _ in 0..8 {
                let _ = parts.next();
            }
            let path = parts.next().unwrap_or("").to_string();
            files.push(GitFile {
                path,
                status: "conflicted".into(),
            });
            continue;
        }
        if let Some(rest) = rec.strip_prefix("? ") {
            files.push(GitFile {
                path: rest.to_string(),
                status: "untracked".into(),
            });
            continue;
        }
        if let Some(_rest) = rec.strip_prefix("! ") {
            continue; // 忽略条目，不展示
        }
        // 其它未知记录（如重命名的原路径残留）忽略
    }
    files
}

fn status_sync(path: &str) -> Result<GitStatus, GitError> {
    let repo_root = resolve_repo_root_sync(path)?;
    let (code, branch, _) = run_git(&args(&["-C", &repo_root, "rev-parse", "--abbrev-ref", "HEAD"]))?;
    let branch = if code == 0 && !branch.trim().is_empty() {
        branch.trim().to_string()
    } else {
        "HEAD".to_string()
    };
    let (code, out, stderr) = run_git(&args(&[
        "-C",
        &repo_root,
        "--no-optional-locks",
        "status",
        "--porcelain=v2",
        "-z",
        "--untracked-files=all",
    ]))?;
    if code != 0 {
        return Err(git_err(if stderr.trim().is_empty() {
            format!("git status 失败（退出码 {code}）")
        } else {
            stderr.trim().to_string()
        }));
    }
    let files = parse_porcelain_v2(&out);
    Ok(GitStatus {
        repo_root,
        branch,
        files,
    })
}

fn init_sync(path: &str) -> Result<GitStatus, GitError> {
    match resolve_repo_root_sync(path) {
        Ok(_) => return Err(git_err("当前目录已经是 Git 仓库，无需初始化")),
        Err(e) if e.code != GitErrorCode::NotARepo => return Err(e),
        Err(_) => {}
    }
    let (code, _, stderr) = run_git(&args(&["-C", path, "init"]))?;
    if code != 0 {
        return Err(git_err(if stderr.trim().is_empty() {
            format!("git init 失败（退出码 {code}）")
        } else {
            stderr.trim().to_string()
        }));
    }
    status_sync(path)
}

fn diff_sync(root: &str, path: &str, kind: &str) -> Result<String, GitError> {
    // 未跟踪文件与空内容对比；其余统一对比 HEAD（覆盖暂存+未暂存）
    let (code, stdout, stderr) = if kind == "untracked" {
        run_git(&args(&["-C", root, "diff", "--no-index", "/dev/null", path]))?
    } else {
        run_git(&args(&["-C", root, "diff", "HEAD", "--", path]))?
    };
    // --no-index 在有差异时退出码为 1，属正常情况
    if code == 0 || (kind == "untracked" && code == 1) {
        Ok(stdout)
    } else {
        Err(git_err(if stderr.trim().is_empty() {
            format!("git diff 失败（退出码 {code}）")
        } else {
            stderr.trim().to_string()
        }))
    }
}

/// 在阻塞线程中执行同步 git 操作，带 60s 超时
async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, GitError> + Send + 'static,
) -> Result<T, GitError> {
    let task = tokio::task::spawn_blocking(f);
    match tokio::time::timeout(Duration::from_secs(60), task).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => Err(git_err(format!("git 任务异常: {e}"))),
        Err(_) => Err(git_err("git 操作超时（60 秒）")),
    }
}

#[tauri::command]
pub async fn git_changes_status(path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || status_sync(&path)).await
}

#[tauri::command]
pub async fn git_changes_init(path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || init_sync(&path)).await
}

#[tauri::command]
pub async fn git_changes_diff(root: String, path: String, kind: String) -> Result<String, GitError> {
    run_blocking(move || diff_sync(&root, &path, &kind)).await
}

// ---------- .git 监听 ----------

/// 规范化路径键（Windows 大小写不敏感）
fn norm_key(p: &Path) -> String {
    p.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// Windows 规范化路径转标准显示路径：`\\?\UNC\...` → `\\...`，`\\?\C:\...` → `C:\...`
fn clean_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    }
}

/// 事件路径是否位于需要排除的“点目录”下（除 .git 外的 . 开头目录，任意深度）。
/// 组件是否目录用路径前缀 is_dir() 判断：`.gitignore`、`.env` 等点文件不排除。
fn path_under_excluded_dot_dir(root: &Path, p: &Path) -> bool {
    let Ok(rel) = p.strip_prefix(root) else {
        return false;
    };
    let mut acc = root.to_path_buf();
    for c in rel.components() {
        if let Component::Normal(n) = c {
            acc.push(n);
            let s = n.to_string_lossy();
            if s.starts_with('.') && s.len() > 1 && s != ".git" && acc.is_dir() {
                return true;
            }
        }
    }
    false
}

#[tauri::command]
pub async fn git_changes_watch_start(
    app: AppHandle,
    state: State<'_, GitWatcherState>,
    root: String,
) -> Result<(), GitError> {
    let repo_root = match run_blocking({
        let root = root.clone();
        move || resolve_repo_root_sync(&root)
    })
    .await
    {
        Ok(r) => PathBuf::from(&r),
        Err(e) if e.code == GitErrorCode::NotARepo => {
            // 非仓库：没有可监听的目标，停掉旧监听后正常返回
            if let Ok(mut guard) = state.0.lock() {
                guard.take();
            }
            return Ok(());
        }
        Err(e) => return Err(e),
    };

    {
        let guard = state.0.lock().map_err(|e| git_err(format!("锁定监听状态失败: {e}")))?;
        if let Some(h) = guard.as_ref() {
            if norm_key(&h.root) == norm_key(&repo_root) {
                return Ok(());
            }
        }
    }
    {
        let mut guard = state.0.lock().map_err(|e| git_err(format!("锁定监听状态失败: {e}")))?;
        guard.take();
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<PathBuf>(1024);
    let tx_watcher = tx.clone();
    let root_for_filter = repo_root.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            if let Some(p) = ev.paths.first() {
                if path_under_excluded_dot_dir(&root_for_filter, p) {
                    return;
                }
                let _ = tx_watcher.try_send(p.clone());
            }
        }
    })
    .map_err(|e| git_err(format!("创建 git 监听失败: {e}")))?;

    watcher
        .watch(&repo_root, RecursiveMode::Recursive)
        .map_err(|e| git_err(format!("监听仓库目录失败 {}: {e}", repo_root.display())))?;

    // 若 git 目录在仓库根之外（linked worktree / 子模块场景），追加监听
    if let Ok((0, git_dir, _)) =
        run_git(&args(&["-C", &repo_root.to_string_lossy(), "rev-parse", "--git-dir"]))
    {
        let gd_raw = git_dir.trim();
        if !gd_raw.is_empty() {
            let gd = if Path::new(gd_raw).is_absolute() {
                PathBuf::from(gd_raw)
            } else {
                repo_root.join(gd_raw)
            };
            if norm_key(&gd) != norm_key(&repo_root) && gd.is_dir() {
                let _ = watcher.watch(&gd, RecursiveMode::Recursive);
            }
        }
    }

    // 防抖任务：300ms 静默后向前端 emit 变更事件
    let handle = app.clone();
    let root_for_task = repo_root.clone();
    tauri::async_runtime::spawn(async move {
        let mut last_event: Option<tokio::time::Instant> = None;
        let mut ticker = tokio::time::interval(Duration::from_millis(150));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(_) => last_event = Some(tokio::time::Instant::now()),
                    None => break,
                },
                _ = ticker.tick() => {
                    if let Some(t) = last_event {
                        if t.elapsed() >= Duration::from_millis(300) {
                            let payload = serde_json::json!({ "root": clean_path(&root_for_task) });
                            let _ = handle.emit("git-changes/changed", payload);
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
        .map_err(|e| git_err(format!("锁定监听状态失败: {e}")))?
        .replace(GitWatchHandle {
            _watcher: watcher,
            _sender: tx,
            root: repo_root,
        });
    Ok(())
}

#[tauri::command]
pub async fn git_changes_watch_stop(state: State<'_, GitWatcherState>) -> Result<(), GitError> {
    let mut guard = state.0.lock().map_err(|e| git_err(format!("锁定监听状态失败: {e}")))?;
    guard.take();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn run_in(dir: &Path, cmd: &[&str]) -> (i32, String, String) {
        let mut all: Vec<String> = vec!["-C".into(), dir.to_string_lossy().into_owned()];
        all.extend(cmd.iter().map(|s| s.to_string()));
        run_git(&all).expect("git 执行失败")
    }

    fn init_committed_repo(dir: &Path) {
        assert_eq!(run_in(dir, &["init"]).0, 0);
        assert_eq!(run_in(dir, &["config", "user.name", "t"]).0, 0);
        assert_eq!(run_in(dir, &["config", "user.email", "t@t"]).0, 0);
        assert_eq!(run_in(dir, &["add", "-A"]).0, 0);
        assert_eq!(run_in(dir, &["commit", "-m", "init"]).0, 0);
    }

    #[test]
    fn parse_porcelain_v2_handles_entries() {
        let mut out = String::new();
        out.push_str("1 M. N... 100644 100644 100644 aaa bbb mod file.txt\0");
        out.push_str("1 .D N... 100644 100644 000000 ccc ddd gone.txt\0");
        out.push_str("1 A. N... 000000 100644 100644 0000000 eee staged.txt\0");
        out.push_str("u UU N... 100644 100644 100644 100644 fff ggg hhh conflict.txt\0");
        out.push_str("? new dir/untracked.txt\0");
        out.push_str("! ignored.txt\0");
        out.push_str("2 R. N... 100644 100644 100644 iii jjj R100 renamed.txt\0");
        out.push_str("old.txt\0");
        let files = parse_porcelain_v2(&out);
        let pairs: Vec<(String, String)> =
            files.iter().map(|f| (f.path.clone(), f.status.clone())).collect();
        assert_eq!(
            pairs,
            vec![
                ("mod file.txt".to_string(), "modified".to_string()),
                ("gone.txt".to_string(), "deleted".to_string()),
                ("staged.txt".to_string(), "added".to_string()),
                ("conflict.txt".to_string(), "conflicted".to_string()),
                ("new dir/untracked.txt".to_string(), "untracked".to_string()),
                ("renamed.txt".to_string(), "renamed".to_string()),
            ]
        );
    }

    #[test]
    fn status_clean_repo_returns_empty() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        let st = status_sync(root.to_str().unwrap()).unwrap();
        assert!(st.files.is_empty());
        assert!(!st.repo_root.is_empty());
        assert!(!st.branch.is_empty());
    }

    #[test]
    fn status_lists_worktree_changes() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        std::fs::write(root.join("b.txt"), "keep").unwrap();
        init_committed_repo(root);
        std::fs::write(root.join("a.txt"), "hello2").unwrap();
        std::fs::remove_file(root.join("b.txt")).unwrap();
        std::fs::write(root.join("c.txt"), "new").unwrap();
        let st = status_sync(root.to_str().unwrap()).unwrap();
        let mut by_path: Vec<(String, String)> =
            st.files.iter().map(|f| (f.path.clone(), f.status.clone())).collect();
        by_path.sort();
        assert_eq!(
            by_path,
            vec![
                ("a.txt".to_string(), "modified".to_string()),
                ("b.txt".to_string(), "deleted".to_string()),
                ("c.txt".to_string(), "untracked".to_string()),
            ]
        );
    }

    #[test]
    fn init_creates_repo_without_commit() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let st = init_sync(root.to_str().unwrap()).unwrap();
        assert!(root.join(".git").is_dir());
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].path, "a.txt");
        assert_eq!(st.files[0].status, "untracked");
        // 仅初始化，不应产生任何提交
        let (code, _, _) = run_in(root, &["rev-parse", "--verify", "HEAD"]);
        assert_ne!(code, 0);
    }

    #[test]
    fn init_twice_errors() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let _ = init_sync(root.to_str().unwrap()).unwrap();
        let err = init_sync(root.to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, GitErrorCode::RepoError);
        assert!(err.message.contains("已经是 Git 仓库"));
    }

    #[test]
    fn status_not_a_repo() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("x.txt"), "x").unwrap();
        let err = status_sync(dir.path().to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, GitErrorCode::NotARepo);
    }

    #[test]
    fn diff_untracked_returns_new_file() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("c.txt"), "hello\n").unwrap();
        let st = init_sync(root.to_str().unwrap()).unwrap();
        let diff = diff_sync(&st.repo_root, "c.txt", "untracked").unwrap();
        assert!(diff.contains("@@ -0,0"));
        assert!(diff.contains("+hello"));
    }

    #[test]
    fn diff_modified_returns_hunks() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        init_committed_repo(root);
        std::fs::write(root.join("a.txt"), "one\ntwo!\n").unwrap();
        let st = status_sync(root.to_str().unwrap()).unwrap();
        let diff = diff_sync(&st.repo_root, "a.txt", "modified").unwrap();
        assert!(diff.contains("@@"));
        assert!(diff.contains("-two"));
        assert!(diff.contains("+two!"));
    }

    #[test]
    fn status_does_not_write_index() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        let index = root.join(".git").join("index");
        let before = std::fs::metadata(&index).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(1100));
        let st = status_sync(root.to_str().unwrap()).unwrap();
        let after = std::fs::metadata(&index).unwrap().modified().unwrap();
        assert_eq!(before, after, "status 不应写入 .git/index");
        assert!(st.files.is_empty());
    }

    #[test]
    fn path_excludes_dot_dirs_but_keeps_git_and_dot_files() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".vs")).unwrap();
        std::fs::create_dir_all(root.join(".vscode")).unwrap();
        std::fs::create_dir_all(root.join("src/.cache")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "x").unwrap();
        std::fs::write(root.join("src/a.txt"), "x").unwrap();
        std::fs::write(root.join(".git/index"), "x").unwrap();

        assert!(path_under_excluded_dot_dir(root, &root.join(".vs/foo")));
        assert!(path_under_excluded_dot_dir(root, &root.join("src/.cache/x")));
        assert!(path_under_excluded_dot_dir(root, &root.join(".vscode/settings.json")));
        assert!(!path_under_excluded_dot_dir(root, &root.join(".git/index")));
        assert!(!path_under_excluded_dot_dir(root, &root.join(".gitignore")));
        assert!(!path_under_excluded_dot_dir(root, &root.join("src/a.txt")));
    }
}
