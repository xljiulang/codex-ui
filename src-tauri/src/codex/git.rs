use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::codex::diff::DiffRow;
use crate::codex::path_util::{clean_path, norm_key, rel_path_of as shared_rel_path_of};
use crate::codex::session_fs::looks_text;
use crate::codex::util::{BlockingError, spawn_blocking_timeout};

/// 单文件大小上限（diff 等全量读入内存的操作），超过直接报错
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// 文件监听防抖窗口：最后一次 watch 文件变化后静默 300ms 才获取 git 变更信息，
/// 避免频繁创建 git 子进程。
const WATCH_DEBOUNCE_MS: u64 = 300;

/// 串行化所有 git 操作：git 子进程会写索引锁文件，并发写会互相覆盖。
/// 锁在 spawn_blocking 任务内部获取并持有到任务真正结束，超时后任务继续执行时
/// 后续操作也会排队等待，避免与后台仍在运行的 git 操作并发读写仓库。
static GIT_OP_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// 启动时探测并缓存的系统 git 可执行文件路径（None 表示未安装）
static GIT_BIN: OnceLock<Option<PathBuf>> = OnceLock::new();

fn git_op_lock() -> &'static Mutex<()> {
    GIT_OP_LOCK.get_or_init(|| Mutex::new(()))
}

/// git 错误码：前端据此渲染不同的空状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GitErrorCode {
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

fn not_a_repo() -> GitError {
    GitError {
        code: GitErrorCode::NotARepo,
        message: "当前目录不在任何 Git 仓库内".to_string(),
    }
}

fn git_missing() -> GitError {
    git_err("未检测到系统 git，请先安装 Git（git-scm.com）或将其加入 PATH")
}

/// 变更状态（前端按小写字符串展示；与 git 状态字母 A/M/D/R/U/C 对应）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[derive(PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Conflicted,
    Untracked,
}

/// 变更文件条目
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFile {
    /// 相对仓库根的路径（正斜杠分隔）
    pub path: String,
    /// added | modified | deleted | renamed | conflicted | untracked
    pub status: FileStatus,
    /// 是否已有暂存区变化（HEAD → 索引）
    pub staged: bool,
    /// 是否含工作区侧变化（索引 → 工作区，含未跟踪文件）
    pub worktree: bool,
}

/// Git 状态（供前端展示）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    /// 仓库根目录（Windows 反斜杠路径）
    pub repo_workspace: String,
    /// 当前分支名；游离 HEAD 或尚未出生时为 "HEAD"
    pub branch: String,
    /// 仓库是否配置了任意远端（拉取/推送可用性的前置条件）
    pub has_remote: bool,
    pub files: Vec<GitFile>,
}

/// 拉取结果（供前端展示）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPullResult {
    /// 拉取后的最新状态
    pub status: GitStatus,
    /// up_to_date | fast_forward | merged
    pub kind: String,
    /// 给用户的中文结果提示
    pub message: String,
}

/// 推送结果（供前端展示）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPushResult {
    /// 推送后的最新状态
    pub status: GitStatus,
    /// pushed | up_to_date
    pub kind: String,
    /// 给用户的中文结果提示
    pub message: String,
}

/// 分支合并结果（供前端展示）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitMergeResult {
    /// 合并后的最新状态
    pub status: GitStatus,
    /// up_to_date | fast_forward | merged
    pub kind: String,
    /// 给用户的中文结果提示
    pub message: String,
}

/// 提交历史条目（供前端展示）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitEntry {
    /// 完整提交哈希
    pub hash: String,
    /// 7 位短哈希
    pub short_hash: String,
    /// 提交标题（首行，空白折叠）
    pub subject: String,
    /// 作者名
    pub author: String,
    /// UNIX 秒时间戳（作者时区）
    pub time_secs: i64,
}

/// 提交内变更文件条目（供前端展示）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitFile {
    /// 相对仓库根的路径（正斜杠分隔；重命名取新路径）
    pub path: String,
    /// added | modified | deleted | renamed | conflicted | untracked
    pub status: FileStatus,
    /// 新增行数（二进制为 0）
    pub insertions: u32,
    /// 删除行数（二进制为 0）
    pub deletions: u32,
    /// 是否二进制文件（git numstat 显示 -）
    pub binary: bool,
}

/// 提交详情（供前端展示；文件统计对比第一个父提交，根提交对比空树）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitDetail {
    /// 完整提交哈希
    pub hash: String,
    /// 7 位短哈希
    pub short_hash: String,
    /// 提交标题（首行）
    pub subject: String,
    /// 完整提交消息（标题 + 正文，保留换行）
    pub body: String,
    /// 作者名
    pub author: String,
    /// 作者邮箱
    pub author_email: String,
    /// 作者 UNIX 秒时间戳（作者时区）
    pub author_time_secs: i64,
    /// 提交者名
    pub committer: String,
    /// 提交者邮箱
    pub committer_email: String,
    /// 提交者 UNIX 秒时间戳（提交者时区）
    pub committer_time_secs: i64,
    /// 父提交完整哈希（第一个为第一父提交；合并提交多个）
    pub parents: Vec<String>,
    /// 变更文件
    pub files: Vec<GitCommitFile>,
}

/// 远端条目（供前端展示）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRemote {
    /// 远端名
    pub name: String,
    /// 拉取地址（remote.<name>.url）；仅配置了推送地址时可能为 null
    pub fetch_url: Option<String>,
    /// 推送地址（remote.<name>.pushUrl；未显式配置时与拉取地址相同）
    pub push_url: Option<String>,
}

/// 远端列表（供前端展示）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRemotes {
    /// 当前分支跟踪/默认使用的远端名；游离 HEAD 或无远端时为 null
    pub current: Option<String>,
    /// 按名称排序的远端列表
    pub remotes: Vec<GitRemote>,
}

/// 文件监听句柄（sender 被 drop 时防抖任务随 rx 关闭退出）
pub struct GitWatchHandle {
    _watcher: RecommendedWatcher,
    _sender: tokio::sync::mpsc::Sender<PathBuf>,
    root: PathBuf,
}

pub struct GitWatcherState(pub Mutex<Option<GitWatchHandle>>);

// ---------- git 可执行文件探测与缓存 ----------

/// 探测系统 git 可执行文件：优先 PATH，回退常见安装目录
fn locate_git() -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            for name in ["git.exe", "git"] {
                let cand = dir.join(name);
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
    }
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "LocalAppData"] {
        if let Ok(base) = std::env::var(var) {
            let base = PathBuf::from(base);
            for sub in ["Git\\cmd\\git.exe", "Programs\\Git\\cmd\\git.exe"] {
                let cand = base.join(sub);
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
    }
    None
}

/// 验证候选 git 可执行文件可用（`git --version`）
fn probe() -> Option<PathBuf> {
    let cand = locate_git()?;
    let ok = git_command(&cand)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    ok.then_some(cand)
}

/// codex-ui 启动时调用一次：确认 git 可用状态并缓存结果。
/// 探测失败后整个会话视为未安装 git（安装后需重启应用）。
pub fn probe_git_at_startup() -> bool {
    let found = probe();
    let ok = found.is_some();
    let _ = GIT_BIN.set(found);
    ok
}

/// 返回缓存（或惰性探测）到的 git 可执行文件路径
fn git_bin() -> Result<&'static Path, GitError> {
    if GIT_BIN.get().is_none() {
        let _ = GIT_BIN.set(probe());
    }
    GIT_BIN
        .get()
        .and_then(|o| o.as_deref())
        .ok_or_else(git_missing)
}

// ---------- git 子进程执行 ----------

/// 创建系统 git 命令：Windows GUI 应用下设置 CREATE_NO_WINDOW，
/// 避免子进程闪现黑色控制台窗口。
fn git_command(git_bin: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new(git_bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// 统一执行 git 子命令，返回 (退出码, stdout, stderr)。
/// 固定英文输出便于解析；GIT_TERMINAL_PROMPT=0 避免在无终端环境挂起。
fn git_output(
    git_bin: &Path,
    root: &str,
    args: &[&str],
) -> Result<(i32, String, String), GitError> {
    let (code, stdout, stderr) = git_output_raw(git_bin, root, args)?;
    Ok((
        code,
        String::from_utf8_lossy(&stdout).into_owned(),
        stderr,
    ))
}

/// 执行 git 子命令并返回原始字节 stdout（内容判定用；lossy 转换会破坏
/// 非法 UTF-8 的探测，不能用于判定文件是否为文本）。
fn git_output_raw(
    git_bin: &Path,
    root: &str,
    args: &[&str],
) -> Result<(i32, Vec<u8>, String), GitError> {
    let out = git_command(git_bin)
        .arg("-C")
        .arg(root)
        .args(args)
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                git_missing()
            } else {
                git_err(format!(
                    "执行 git {} 失败: {e}",
                    args.first().copied().unwrap_or("")
                ))
            }
        })?;
    Ok((
        out.status.code().unwrap_or(-1),
        out.stdout,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

/// 执行 git 子命令并校验退出码：成功返回 stdout，失败经 `map_error` 映射为中文提示
fn git_run_mapped(
    git_bin: &Path,
    root: &str,
    args: &[&str],
    map_error: impl Fn(&str) -> String,
) -> Result<String, GitError> {
    let (code, stdout, stderr) = git_output(git_bin, root, args)?;
    if code != 0 {
        let combined = format!("{stdout}\n{stderr}");
        return Err(git_err(map_error(&combined)));
    }
    Ok(stdout)
}

// ---------- 仓库发现 ----------

/// 仓库信息：工作区根与 git 目录
struct RepoInfo {
    workdir: PathBuf,
    git_dir: PathBuf,
}

/// 向上发现 Git 仓库（`git rev-parse`）；非仓库返回 `NotARepo`
fn git_rev_parse(root: &str) -> Result<RepoInfo, GitError> {
    let git_bin = git_bin()?;
    git_rev_parse_with(&git_bin, root)
}

fn git_rev_parse_with(git_bin: &Path, root: &str) -> Result<RepoInfo, GitError> {
    let (code, stdout, stderr) = git_output(
        git_bin,
        root,
        &[
            "rev-parse",
            "--show-toplevel",
            "--git-dir",
            "--is-inside-work-tree",
        ],
    )?;
    if code != 0 {
        let c = format!("{stdout}\n{stderr}").to_lowercase();
        if c.contains("not a git repository") || c.contains("no git repository") {
            return Err(not_a_repo());
        }
        return Err(git_err(format!("无法打开 Git 仓库: {}", stderr.trim())));
    }
    let mut lines = stdout.lines();
    let workdir = lines.next().ok_or_else(|| git_err("git rev-parse 输出不完整"))?;
    let git_dir = lines.next().ok_or_else(|| git_err("git rev-parse 输出不完整"))?;
    let inside = lines.next().unwrap_or("true");
    if inside.trim() != "true" {
        return Err(not_a_repo());
    }
    Ok(RepoInfo {
        workdir: PathBuf::from(workdir),
        git_dir: PathBuf::from(git_dir),
    })
}

/// 路径是否包含 node_modules 组件（任意深度，大小写不敏感）
fn has_node_modules_component(path: &str) -> bool {
    path.split('/').any(|c| c.eq_ignore_ascii_case("node_modules"))
}

// ---------- 状态 ----------

/// 合并同一路径的多个状态（暂存 + 未暂存），展示优先级：
/// conflicted > deleted > modified > renamed/added > untracked
fn merge_file(
    files: &mut Vec<GitFile>,
    index_of: &mut HashMap<String, usize>,
    path: String,
    status: FileStatus,
    staged: bool,
    worktree: bool,
) {
    let prio = |s: FileStatus| match s {
        FileStatus::Conflicted => 5,
        FileStatus::Deleted => 4,
        FileStatus::Modified => 3,
        FileStatus::Renamed | FileStatus::Added => 2,
        FileStatus::Untracked => 1,
    };
    let norm = path.to_lowercase();
    if let Some(&i) = index_of.get(&norm) {
        if prio(status) > prio(files[i].status) {
            files[i].status = status;
        }
        files[i].staged |= staged;
        files[i].worktree |= worktree;
    } else {
        index_of.insert(norm, files.len());
        files.push(GitFile {
            path,
            status,
            staged,
            worktree,
        });
    }
}

/// porcelain XY 状态字母 → FileStatus（'?' 未跟踪单独处理）
fn letter_status(c: char) -> Option<FileStatus> {
    match c {
        'A' => Some(FileStatus::Added),
        'D' => Some(FileStatus::Deleted),
        'M' | 'T' => Some(FileStatus::Modified),
        'R' | 'C' => Some(FileStatus::Renamed),
        'U' => Some(FileStatus::Conflicted),
        _ => None,
    }
}

/// 当前分支短名（游离 HEAD 或 HEAD 未出生/失败时为 "HEAD"）
fn git_current_branch_with(git_bin: &Path, root: &str) -> Result<String, GitError> {
    let (code, stdout, _) = git_output(git_bin, root, &["symbolic-ref", "--short", "-q", "HEAD"])?;
    let name = stdout.trim();
    if code == 0 && !name.is_empty() {
        Ok(name.to_string())
    } else {
        Ok("HEAD".to_string())
    }
}

/// 仓库是否配置了任意远端
fn git_has_remote_with(git_bin: &Path, root: &str) -> Result<bool, GitError> {
    let (code, stdout, _) = git_output(git_bin, root, &["remote"])?;
    if code != 0 {
        return Err(git_err("读取远端列表失败"));
    }
    Ok(!stdout.trim().is_empty())
}

fn git_status(root: &str) -> Result<GitStatus, GitError> {
    let git_bin = git_bin()?;
    git_status_with(&git_bin, root)
}

/// 状态（标准 porcelain）：默认开启重命名检测；`-z` 下条目为 `XY 路径`，
/// 重命名/复制为双路径（`新路径\0旧路径\0`），展示路径取新路径。
fn git_status_with(git_bin: &Path, root: &str) -> Result<GitStatus, GitError> {
    let repo = git_rev_parse_with(git_bin, root)?;
    let workdir = clean_path(&repo.workdir);
    let branch = git_current_branch_with(git_bin, root)?;
    let has_remote = git_has_remote_with(git_bin, root)?;

    let (code, stdout, stderr) = git_output(
        git_bin,
        root,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "-z",
            "--untracked-files=all",
        ],
    )?;
    if code != 0 {
        return Err(git_err(format!("git status 失败: {}", stderr.trim())));
    }

    let mut files: Vec<GitFile> = Vec::new();
    let mut index_of: HashMap<String, usize> = HashMap::new();
    let tokens: Vec<&str> = stdout.split('\0').collect();
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        // 合法条目至少 "XY " + 一个字符的路径
        if tok.len() < 4 || !tok.as_bytes()[2].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let x = tok.as_bytes()[0] as char;
        let y = tok.as_bytes()[1] as char;
        let path = &tok[3..];
        let is_rename = x == 'R' || x == 'C' || y == 'R' || y == 'C';
        let orig = if is_rename && i + 1 < tokens.len() {
            let o = tokens[i + 1].to_string();
            i += 1;
            Some(o)
        } else {
            None
        };
        // 无条件隐藏 node_modules（含重命名任一侧路径）
        if has_node_modules_component(path)
            || orig.as_deref().map(has_node_modules_component).unwrap_or(false)
        {
            i += 1;
            continue;
        }
        if x == '?' && y == '?' {
            merge_file(
                &mut files,
                &mut index_of,
                path.to_string(),
                FileStatus::Untracked,
                false,
                true,
            );
        } else {
            if let Some(st) = letter_status(x) {
                merge_file(&mut files, &mut index_of, path.to_string(), st, true, false);
            }
            if let Some(st) = letter_status(y) {
                merge_file(&mut files, &mut index_of, path.to_string(), st, false, true);
            }
        }
        i += 1;
    }

    files.sort_by_key(|a| a.path.to_lowercase());
    Ok(GitStatus {
        repo_workspace: workdir,
        branch,
        has_remote,
        files,
    })
}

/// 默认 .gitignore 模板（初始化时创建，已存在则跳过；LF、UTF-8、无 BOM）
const DEFAULT_GITIGNORE: &str = r#"# 操作系统
Thumbs.db
Desktop.ini
.DS_Store

# JetBrains / IntelliJ（IDEA、PyCharm、WebStorm、GoLand 等）
.idea/
*.iml

# Eclipse
.classpath
.project
.settings/

# Visual Studio
.vs/
*.user
*.suo
*.userosscache
*.sln.docstates
*.VC.db
*.VC.opendb

# VS Code
.vscode/
*.code-workspace

# Xcode
xcuserdata/
DerivedData/
*.xcuserstate

# 依赖与构建产物
node_modules/
dist/
build/
target/
bin/
obj/
*.log
"#;

/// 默认 .gitattributes 模板（初始化时创建，已存在则跳过；不强制 LF）
const DEFAULT_GITATTRIBUTES: &str = "* text=auto\n";

fn git_init(root: &str) -> Result<GitStatus, GitError> {
    match git_rev_parse(root) {
        Ok(_) => return Err(git_err("当前目录已经是 Git 仓库，无需初始化")),
        Err(e) if e.code != GitErrorCode::NotARepo => return Err(e),
        Err(_) => {}
    }
    let git_bin = git_bin()?;
    let (code, _, stderr) = git_output(&git_bin, root, &["init", "-b", "main"])?;
    if code != 0 {
        return Err(git_err(format!("git 初始化失败: {}", stderr.trim())));
    }
    let repo = git_rev_parse_with(&git_bin, root)?;
    // 默认创建 .gitignore / .gitattributes（已存在则跳过，保留用户内容）
    let gitignore = repo.workdir.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(&gitignore, DEFAULT_GITIGNORE)
            .map_err(|e| git_err(format!("写入 .gitignore 失败: {e}")))?;
    }
    let gitattributes = repo.workdir.join(".gitattributes");
    if !gitattributes.exists() {
        std::fs::write(&gitattributes, DEFAULT_GITATTRIBUTES)
            .map_err(|e| git_err(format!("写入 .gitattributes 失败: {e}")))?;
    }
    git_status_with(&git_bin, root)
}

// ---------- 分支管理 ----------

/// Git 分支列表（供前端展示）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitBranches {
    /// 当前分支名；游离 HEAD 或尚未出生时为 "HEAD"
    pub current: String,
    /// 本地分支名（按名称排序）
    pub branches: Vec<String>,
    /// 远端跟踪分支短名（refs/remotes/<remote>/<branch>，如 origin/main，按名称排序）
    pub remote_branches: Vec<String>,
    /// 当前分支的上游「远端/分支」短名（如 origin/main）；无上游时为 null
    pub current_upstream: Option<String>,
}

/// 分支名合法性校验（与 git check-ref-format 近似）
fn validate_branch_name(name: &str) -> Result<(), GitError> {
    if name.is_empty() {
        return Err(git_err("分支名不能为空"));
    }
    if name.starts_with('-') || name.starts_with('/') || name.ends_with('/') || name.ends_with('.') {
        return Err(git_err("分支名不能以 - 或 / 开头，也不能以 / 或 . 结尾"));
    }
    if name.contains("..")
        || name.contains(' ')
        || name.contains(['~', '^', ':', '?', '*', '[', '\\'])
        || name.contains("@{")
    {
        return Err(git_err("分支名包含非法字符"));
    }
    if name == "@" {
        return Err(git_err("分支名不合法"));
    }
    Ok(())
}

/// 分支列表（`git branch` / `git branch -r`）
fn git_branch(root: &str) -> Result<GitBranches, GitError> {
    let git_bin = git_bin()?;
    git_branch_with(&git_bin, root)
}

fn git_branch_with(git_bin: &Path, root: &str) -> Result<GitBranches, GitError> {
    git_rev_parse_with(git_bin, root)?;
    let current = git_current_branch_with(git_bin, root)?;
    let mut branches =
        git_list_refs_with(git_bin, root, &["branch", "--format=%(refname:short)"])?;
    let mut remote_branches = git_list_refs_with(
        git_bin,
        root,
        &["branch", "-r", "--format=%(refname:short)"],
    )?
    .into_iter()
    .filter(|b| !b.ends_with("/HEAD"))
    .collect::<Vec<String>>();
    branches.sort_by_key(|a| a.to_lowercase());
    remote_branches.sort_by_key(|a| a.to_lowercase());
    let current_upstream = if current == "HEAD" {
        None
    } else {
        git_upstream_with(git_bin, root, &current)?
    };
    Ok(GitBranches {
        current,
        branches,
        remote_branches,
        current_upstream,
    })
}

/// 执行会输出多行的 git 列表命令并收集非空行
fn git_list_refs_with(
    git_bin: &Path,
    root: &str,
    args: &[&str],
) -> Result<Vec<String>, GitError> {
    let (code, stdout, stderr) = git_output(git_bin, root, args)?;
    if code != 0 {
        return Err(git_err(format!("git {} 失败: {}", args[0], stderr.trim())));
    }
    Ok(stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// 当前分支上游「远端/分支」短名（无上游/游离 HEAD 时为 None）
fn git_upstream_with(
    git_bin: &Path,
    root: &str,
    _branch: &str,
) -> Result<Option<String>, GitError> {
    let (code, stdout, _) = git_output(
        git_bin,
        root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"],
    )?;
    if code != 0 {
        return Ok(None);
    }
    let up = stdout.trim();
    if up.is_empty() {
        Ok(None)
    } else {
        Ok(Some(up.to_string()))
    }
}

/// 引用是否存在
fn git_ref_exists_with(git_bin: &Path, root: &str, refname: &str) -> Result<bool, GitError> {
    let (code, _, _) = git_output(git_bin, root, &["show-ref", "--verify", "--quiet", refname])?;
    Ok(code == 0)
}

/// HEAD 是否已有提交（未出生 → false）
fn git_head_exists_with(git_bin: &Path, root: &str) -> Result<bool, GitError> {
    let (code, _, _) = git_output(git_bin, root, &["rev-parse", "--verify", "--quiet", "HEAD"])?;
    Ok(code == 0)
}

/// 创建分支（`git branch <name>`）
fn git_branch_create(root: &str, name: &str) -> Result<GitStatus, GitError> {
    let name = name.trim();
    validate_branch_name(name)?;
    let git_bin = git_bin()?;
    git_rev_parse_with(&git_bin, root)?;
    if !git_head_exists_with(&git_bin, root)? {
        return Err(git_err("仓库还没有任何提交，无法创建分支"));
    }
    if git_ref_exists_with(&git_bin, root, &format!("refs/heads/{name}"))? {
        return Err(git_err(format!("分支 {name} 已存在")));
    }
    git_run_mapped(&git_bin, root, &["branch", name], |combined| {
        let trimmed = combined.trim();
        if trimmed.is_empty() {
            format!("创建分支 {name} 失败")
        } else {
            trimmed.to_string()
        }
    })?;
    git_status_with(&git_bin, root)
}

/// 删除分支（`git branch -D`，无条件删除：不校验是否合并）
fn git_branch_delete(root: &str, name: &str) -> Result<GitStatus, GitError> {
    let name = name.trim();
    let git_bin = git_bin()?;
    git_rev_parse_with(&git_bin, root)?;
    let current = git_current_branch_with(&git_bin, root)?;
    if current == name {
        return Err(git_err("不能删除当前所在分支"));
    }
    if !git_ref_exists_with(&git_bin, root, &format!("refs/heads/{name}"))? {
        return Err(git_err(format!("分支 {name} 不存在")));
    }
    git_run_mapped(&git_bin, root, &["branch", "-D", name], branch_delete_failure_message)?;
    git_status_with(&git_bin, root)
}

/// 删除分支失败提示映射
fn branch_delete_failure_message(combined: &str) -> String {
    let trimmed = combined.trim();
    if trimmed.is_empty() {
        "删除分支失败".to_string()
    } else {
        trimmed.to_string()
    }
}

// ---------- 分支切换 ----------

/// 切换分支（`git switch <name>`，标准语义：无冲突即可切换，未提交改动随分支携带）
fn git_switch(root: &str, name: &str) -> Result<GitStatus, GitError> {
    let name = name.trim();
    validate_branch_name(name)?;
    let git_bin = git_bin()?;
    git_rev_parse_with(&git_bin, root)?;
    let current = git_current_branch_with(&git_bin, root)?;
    if current == name {
        return Err(git_err(format!("当前已在分支 {name}")));
    }
    if !git_ref_exists_with(&git_bin, root, &format!("refs/heads/{name}"))? {
        return Err(git_err(format!("分支 {name} 不存在")));
    }
    let (code, stdout, stderr) = git_output(&git_bin, root, &["switch", name])?;
    if code != 0 {
        return Err(git_err(switch_failure_message(
            &format!("{stdout}\n{stderr}"),
            name,
        )));
    }
    git_status_with(&git_bin, root)
}

/// 提取 git 错误输出中以 `\t` 前缀列出的文件列表，用中文顿号连接
fn extract_listed_paths(combined: &str) -> String {
    let mut paths: Vec<String> = Vec::new();
    let mut in_list = false;
    for line in combined.lines() {
        let c = line.to_lowercase();
        if c.contains("would be overwritten") || c.contains("untracked working tree files") {
            in_list = true;
            continue;
        }
        if in_list {
            if line.starts_with('\t') {
                paths.push(line.trim().to_string());
            } else if !line.is_empty() {
                break;
            }
        }
    }
    if paths.is_empty() {
        "<未知路径>".to_string()
    } else {
        paths.join("、")
    }
}

/// 切换分支失败提示映射
fn switch_failure_message(combined: &str, name: &str) -> String {
    let c = combined.to_lowercase();
    if c.contains("your local changes to the following files would be overwritten") {
        format!(
            "本地修改与目标分支冲突，无法切换: {}（请先提交或还原）",
            extract_listed_paths(combined)
        )
    } else if c.contains("untracked working tree files would be overwritten") {
        format!("未跟踪文件将被覆盖: {}", extract_listed_paths(combined))
    } else if c.contains("already on") {
        format!("当前已在分支 {name}")
    } else {
        let trimmed = combined.trim();
        if trimmed.is_empty() {
            format!("切换分支 {name} 失败")
        } else {
            trimmed.to_string()
        }
    }
}


// ---------- 变更文件操作 ----------

/// 校验并规范化仓库相对路径：拒绝绝对路径、`.`/`..`、盘符前缀、`.git` 与 node_modules 组件
fn validate_rel_path(rel: &str) -> Result<String, GitError> {
    let rel = rel.replace('\\', "/");
    if rel.is_empty() {
        return Err(git_err("文件路径不能为空"));
    }
    for c in Path::new(&rel).components() {
        match c {
            Component::Normal(n) => {
                let s = n.to_string_lossy();
                if s == ".git" {
                    return Err(git_err("不支持对 .git 内文件操作"));
                }
                if s.eq_ignore_ascii_case("node_modules") {
                    return Err(git_err("不支持对 node_modules 内文件操作"));
                }
            }
            _ => return Err(git_err("文件路径不合法")),
        }
    }
    Ok(rel)
}

/// 从 `p` 的父目录开始逐级删除空目录，直到仓库根 `workdir`
fn remove_empty_parents(workdir: &Path, p: &Path) {
    let mut parent = p.parent();
    while let Some(d) = parent {
        if d == workdir {
            break;
        }
        if std::fs::remove_dir(d).is_err() {
            break;
        }
        parent = d.parent();
    }
}

/// 目录前缀下的变更文件（含精确匹配；大小写不敏感，与 Windows 路径规则一致）
fn files_under(files: &[GitFile], dir: &str) -> Vec<GitFile> {
    let dir_l = dir.trim_end_matches('/').to_lowercase();
    let prefix_l = format!("{dir_l}/");
    files
        .iter()
        .filter(|f| {
            let p = f.path.to_lowercase();
            p == dir_l || p.starts_with(&prefix_l)
        })
        .cloned()
        .collect()
}

/// 暂存（`git add -- <rel>`；目录递归，未跟踪即添加跟踪，删除同样暂存）
fn git_add(root: &str, rel: &str) -> Result<GitStatus, GitError> {
    let rel = validate_rel_path(rel)?;
    let git_bin = git_bin()?;
    let repo = git_rev_parse_with(&git_bin, root)?;
    // 保留超大单文件保护（git 本身可处理大文件，此处仅前置拦截）
    let abs = repo.workdir.join(&rel);
    if abs.is_file()
        && std::fs::metadata(&abs)
            .map(|m| m.len() > MAX_FILE_BYTES)
            .unwrap_or(false)
    {
        return Err(git_err(format!(
            "文件过大（超过 {} MB），无法暂存: {}",
            MAX_FILE_BYTES / (1024 * 1024),
            clean_path(&abs)
        )));
    }
    git_run_mapped(&git_bin, root, &["add", "--", &rel], git_add_failure_message)?;
    git_status_with(&git_bin, root)
}

/// 全部暂存（`git add -A`）
fn git_add_all(root: &str) -> Result<GitStatus, GitError> {
    let git_bin = git_bin()?;
    git_rev_parse_with(&git_bin, root)?;
    git_run_mapped(&git_bin, root, &["add", "-A"], git_add_failure_message)?;
    git_status_with(&git_bin, root)
}

/// 取消暂存（`git restore --staged -- <rel>`）
fn git_unstage(root: &str, rel: &str) -> Result<GitStatus, GitError> {
    let rel = validate_rel_path(rel)?;
    let git_bin = git_bin()?;
    git_rev_parse_with(&git_bin, root)?;
    if git_head_exists_with(&git_bin, root)? {
        git_run_mapped(&git_bin, root, &["restore", "--staged", "--", &rel], |combined| {
            let trimmed = combined.trim();
            if trimmed.is_empty() {
                "取消暂存失败".to_string()
            } else {
                trimmed.to_string()
            }
        })?;
    } else {
        // HEAD 未出生：`git restore --staged` 依赖 HEAD，改用 `git rm --cached`
        git_run_mapped(&git_bin, root, &["rm", "--cached", "-f", "-q", "--", &rel], |combined| {
            let trimmed = combined.trim();
            if trimmed.is_empty() {
                "取消暂存失败".to_string()
            } else {
                trimmed.to_string()
            }
        })?;
    }
    git_status_with(&git_bin, root)
}

/// 全部取消暂存：`git reset -q`；HEAD 未出生时 `git rm --cached -r -q -- .`
fn git_unstage_all(root: &str) -> Result<GitStatus, GitError> {
    let git_bin = git_bin()?;
    git_rev_parse_with(&git_bin, root)?;
    if git_head_exists_with(&git_bin, root)? {
        git_run_mapped(&git_bin, root, &["reset", "-q"], |combined| {
            let trimmed = combined.trim();
            if trimmed.is_empty() {
                "取消暂存失败".to_string()
            } else {
                trimmed.to_string()
            }
        })?;
    } else {
        git_run_mapped(
            &git_bin,
            root,
            &["rm", "--cached", "-r", "-q", "--", "."],
            |combined| {
                let trimmed = combined.trim();
                if trimmed.is_empty() {
                    "取消暂存失败".to_string()
                } else {
                    trimmed.to_string()
                }
            },
        )?;
    }
    git_status_with(&git_bin, root)
}

/// 索引中是否存在该路径
fn git_index_has_with(git_bin: &Path, root: &str, rel: &str) -> Result<bool, GitError> {
    let (code, _, _) = git_output(git_bin, root, &["ls-files", "--error-unmatch", "--", rel])?;
    Ok(code == 0)
}

/// 还原单个文件：已跟踪 → `git restore --staged --worktree`（丢弃暂存+工作区改动）；
/// 未跟踪 → `git clean -f` 删除
fn git_restore_one_with(
    git_bin: &Path,
    workdir: &Path,
    root: &str,
    rel: &str,
) -> Result<(), GitError> {
    if git_index_has_with(git_bin, root, rel)? {
        git_run_mapped(
            git_bin,
            root,
            &["restore", "--staged", "--worktree", "--", rel],
            git_restore_failure_message,
        )?;
    } else {
        git_run_mapped(
            git_bin,
            root,
            &["clean", "-f", "-q", "--", rel],
            git_clean_failure_message,
        )?;
        remove_empty_parents(workdir, &workdir.join(rel));
    }
    Ok(())
}

/// 还原：支持文件或目录；目录 = 其下所有变更文件丢弃（已跟踪恢复 HEAD，未跟踪删除）
fn git_restore(root: &str, rel: &str) -> Result<GitStatus, GitError> {
    let rel = validate_rel_path(rel)?;
    let git_bin = git_bin()?;
    let repo = git_rev_parse_with(&git_bin, root)?;
    if repo.workdir.join(&rel).is_dir() {
        let st = git_status_with(&git_bin, root)?;
        let rels: Vec<String> = files_under(&st.files, &rel)
            .into_iter()
            .map(|f| f.path)
            .collect();
        for r in &rels {
            let r = validate_rel_path(r)?;
            git_restore_one_with(&git_bin, &repo.workdir, root, &r)?;
        }
    } else {
        git_restore_one_with(&git_bin, &repo.workdir, root, &rel)?;
    }
    git_status_with(&git_bin, root)
}

/// 删除文件：已跟踪 `git rm -f`（工作区+索引），未跟踪 `git clean -f`；
/// 逐级清理空目录；不支持目录
fn git_rm(root: &str, rel: &str) -> Result<GitStatus, GitError> {
    let rel = validate_rel_path(rel)?;
    let git_bin = git_bin()?;
    let repo = git_rev_parse_with(&git_bin, root)?;
    let abs = repo.workdir.join(&rel);
    if abs.is_dir() {
        return Err(git_err("不支持删除目录"));
    }
    if git_index_has_with(&git_bin, root, &rel)? {
        git_run_mapped(&git_bin, root, &["rm", "-f", "--", &rel], git_rm_failure_message)?;
    } else {
        git_run_mapped(
            &git_bin,
            root,
            &["clean", "-f", "-q", "--", &rel],
            git_clean_failure_message,
        )?;
    }
    remove_empty_parents(&repo.workdir, &abs);
    git_status_with(&git_bin, root)
}

/// 添加到 .gitignore：文件追加 `/路径`，目录追加 `/路径/`（不存在则创建，幂等跳过重复条目）
fn git_ignore(root: &str, rel: &str) -> Result<GitStatus, GitError> {
    let git_bin = git_bin()?;
    let repo = git_rev_parse_with(&git_bin, root)?;
    let rel = validate_rel_path(rel)?;
    let pattern = if repo.workdir.join(&rel).is_dir() {
        format!("/{}/", rel.trim_end_matches('/'))
    } else {
        format!("/{rel}")
    };
    let plain = pattern.trim_start_matches('/');
    let gitignore = repo.workdir.join(".gitignore");
    let existing = std::fs::read_to_string(&gitignore).unwrap_or_default();
    let already = existing.lines().any(|l| {
        let l = l.strip_suffix('\r').unwrap_or(l);
        l == pattern || l == plain
    });
    if !already {
        let mut out = existing;
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&pattern);
        out.push('\n');
        std::fs::write(&gitignore, out)
            .map_err(|e| git_err(format!("写入 .gitignore 失败: {e}")))?;
    }
    git_status_with(&git_bin, root)
}

/// `git add` 失败提示映射
fn git_add_failure_message(combined: &str) -> String {
    let trimmed = combined.trim();
    if trimmed.is_empty() {
        "git add 失败".to_string()
    } else {
        trimmed.to_string()
    }
}

/// `git restore` 失败提示映射
fn git_restore_failure_message(combined: &str) -> String {
    let c = combined.to_lowercase();
    if c.contains("did not match") || c.contains("pathspec") {
        "文件不在仓库中，无法还原".to_string()
    } else {
        let trimmed = combined.trim();
        if trimmed.is_empty() {
            "git restore 失败".to_string()
        } else {
            trimmed.to_string()
        }
    }
}

/// `git clean` 失败提示映射
fn git_clean_failure_message(combined: &str) -> String {
    let c = combined.to_lowercase();
    if c.contains("cannot clean") || c.contains("not remove") {
        "无法删除未跟踪文件".to_string()
    } else {
        let trimmed = combined.trim();
        if trimmed.is_empty() {
            "git clean 失败".to_string()
        } else {
            trimmed.to_string()
        }
    }
}

/// `git rm` 失败提示映射
fn git_rm_failure_message(combined: &str) -> String {
    let trimmed = combined.trim();
    if trimmed.is_empty() {
        "git rm 失败".to_string()
    } else {
        trimmed.to_string()
    }
}

// ---------- 提交与拉取 ----------


/// 提交（`git commit -m <message>`）：仅提交已暂存更改，未暂存保留在工作区
fn git_commit(root: &str, message: &str) -> Result<GitStatus, GitError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(git_err("提交消息不能为空"));
    }
    let git_bin = git_bin()?;
    git_rev_parse_with(&git_bin, root)?;
    // 未解决冲突 → 拒绝
    let (code, unmerged, _) = git_output(&git_bin, root, &["ls-files", "-u"])?;
    if code != 0 {
        return Err(git_err("读取索引失败"));
    }
    if !unmerged.trim().is_empty() {
        return Err(git_err("仓库存在未解决的合并冲突，请先解决冲突后再提交"));
    }
    // 无已暂存更改 → 拒绝（`git diff --cached --quiet` 退出码 0 表示无差异）
    let (code, _, _) = git_output(&git_bin, root, &["diff", "--cached", "--quiet"])?;
    if code == 0 {
        return Err(git_err("没有已暂存的更改，请先暂存文件"));
    }
    let (code, stdout, stderr) = git_output(&git_bin, root, &["commit", "-m", message])?;
    if code != 0 {
        let combined = format!("{stdout}\n{stderr}");
        let c = combined.to_lowercase();
        if c.contains("please tell me who you are")
            || c.contains("unable to auto-detect email address")
            || c.contains("user.name")
        {
            return Err(git_err("未配置 Git 用户信息（user.name / user.email），请先在仓库配置后重试"));
        }
        return Err(git_err(commit_failure_message(&combined)));
    }
    git_status_with(&git_bin, root)
}

/// 提交失败提示映射
fn commit_failure_message(combined: &str) -> String {
    let trimmed = combined.trim();
    if trimmed.is_empty() {
        "提交失败".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 提交历史（`git log -z --format=%H%x00%an%x00%at%x00%s`）。
/// `before` 为续页游标（上一批最后一条的完整 hash），从游标的父提交继续向后取；
/// 无效游标或仓库尚无提交时返回空列表。
fn git_log(root: &str, limit: usize, before: Option<String>) -> Result<Vec<GitCommitEntry>, GitError> {
    let limit = limit.clamp(1, 200);
    let git_bin = git_bin()?;
    git_rev_parse_with(&git_bin, root)?;
    let mut args: Vec<String> = vec![
        "log".to_string(),
        "-z".to_string(),
        "--format=%H%x00%an%x00%at%x00%s".to_string(),
        "-n".to_string(),
        limit.to_string(),
    ];
    if let Some(b) = before {
        args.push(format!("{b}^"));
    }
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (code, stdout, _) = git_output(&git_bin, root, &arg_refs)?;
    if code != 0 {
        return Ok(Vec::new());
    }
    let fields: Vec<&str> = stdout.split('\0').collect();
    let mut out: Vec<GitCommitEntry> = Vec::new();
    let mut i = 0;
    while i + 3 < fields.len() {
        let hash = fields[i];
        if hash.is_empty() {
            break;
        }
        out.push(GitCommitEntry {
            hash: hash.to_string(),
            short_hash: hash.chars().take(7).collect(),
            subject: fields[i + 3].to_string(),
            author: fields[i + 1].to_string(),
            time_secs: fields[i + 2].parse().unwrap_or(0),
        });
        i += 4;
    }
    Ok(out)
}

/// 空树对象哈希（固定值），作为根提交（无父提交）的 diff 基准
const EMPTY_TREE_HASH: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// 提交的父哈希列表（`git log -1 --format=%P`，空格分隔；根提交为空）
fn commit_parents(git_bin: &Path, root: &str, hash: &str) -> Result<Vec<String>, GitError> {
    let (code, stdout, stderr) = git_output(
        git_bin,
        root,
        &["log", "-1", "--format=%P", hash],
    )?;
    if code != 0 {
        let combined = format!("{stdout}\n{stderr}");
        return Err(git_err(format!(
            "无法读取提交详情: {}",
            combined.trim()
        )));
    }
    Ok(stdout.split_whitespace().map(|s| s.to_string()).collect())
}

/// diff 基准：第一个父提交；根提交用空树
fn commit_base_from_parents(parents: &[String]) -> String {
    parents
        .first()
        .cloned()
        .unwrap_or_else(|| EMPTY_TREE_HASH.to_string())
}

/// numstat 重命名路径 `old => new` 取新路径（git diff --numstat -M 输出形式）
fn rename_new_path(path: &str) -> &str {
    path.rsplit_once(" => ").map(|(_, new)| new).unwrap_or(path)
}

/// name-status 状态字母 → FileStatus（-M 下可能出现 R100；T 视为修改）
fn commit_status_letter(status: &str) -> FileStatus {
    match status.chars().next() {
        Some('A') => FileStatus::Added,
        Some('D') => FileStatus::Deleted,
        Some('R') => FileStatus::Renamed,
        _ => FileStatus::Modified,
    }
}

/// 提交变更文件列表：numstat 取增删行数，name-status 取状态字母，按新路径合并
fn git_commit_files(
    git_bin: &Path,
    root: &str,
    base: &str,
    hash: &str,
) -> Result<Vec<GitCommitFile>, GitError> {
    let (code, numstat, stderr) = git_output(
        git_bin,
        root,
        &["diff", "--numstat", "-M", "--no-ext-diff", base, hash],
    )?;
    if code != 0 {
        return Err(git_err(format!(
            "读取提交文件统计失败: {}",
            stderr.trim()
        )));
    }
    let (code2, namestat, stderr2) = git_output(
        git_bin,
        root,
        &["diff", "--name-status", "-M", "--no-ext-diff", base, hash],
    )?;
    if code2 != 0 {
        return Err(git_err(format!(
            "读取提交文件状态失败: {}",
            stderr2.trim()
        )));
    }
    let mut counts: HashMap<String, (u32, u32, bool)> = HashMap::new();
    for line in numstat.lines() {
        let mut parts = line.split('\t');
        let added = parts.next().unwrap_or("");
        let deleted = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        let path = rename_new_path(path);
        let binary = added == "-" || deleted == "-";
        let (a, d) = if binary {
            (0, 0)
        } else {
            (
                added.parse().unwrap_or(0),
                deleted.parse().unwrap_or(0),
            )
        };
        counts.insert(path.to_string(), (a, d, binary));
    }
    let mut files: Vec<GitCommitFile> = Vec::new();
    for line in namestat.lines() {
        let mut parts = line.split('\t');
        let status = parts.next().unwrap_or("");
        // 重命名行含旧/新两个路径字段（R100\told\tnew），取最后一个为新路径
        let rest: Vec<&str> = parts.collect();
        let path = rest.last().copied().unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        let path = rename_new_path(path);
        let (insertions, deletions, binary) =
            counts.remove(path).unwrap_or((0, 0, false));
        files.push(GitCommitFile {
            path: path.to_string(),
            status: commit_status_letter(status),
            insertions,
            deletions,
            binary,
        });
    }
    Ok(files)
}

/// 提交详情（元信息 + 变更文件统计；对比第一个父提交，根提交对比空树）
fn git_commit_detail(root: &str, hash: &str) -> Result<GitCommitDetail, GitError> {
    let hash = hash.trim();
    if hash.is_empty() {
        return Err(git_err("提交哈希不能为空"));
    }
    let git_bin = git_bin()?;
    git_rev_parse_with(&git_bin, root)?;
    let (code, stdout, stderr) = git_output(
        &git_bin,
        root,
        &[
            "log",
            "-1",
            "--format=%H%x00%an%x00%ae%x00%at%x00%cn%x00%ce%x00%ct%x00%P%x00%B",
            hash,
        ],
    )?;
    if code != 0 {
        let combined = format!("{stdout}\n{stderr}");
        return Err(git_err(format!(
            "无法读取提交详情: {}",
            combined.trim()
        )));
    }
    let fields: Vec<&str> = stdout.split('\0').collect();
    if fields.len() < 9 || fields[0].is_empty() {
        return Err(git_err("无法解析提交详情（输出不完整）"));
    }
    let hash_full = fields[0].to_string();
    let parents: Vec<String> = fields[7]
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    let body = fields[8].trim_end().to_string();
    let subject = body.lines().next().unwrap_or("").to_string();
    let base = commit_base_from_parents(&parents);
    let files = git_commit_files(&git_bin, root, &base, &hash_full)?;
    Ok(GitCommitDetail {
        short_hash: hash_full.chars().take(7).collect(),
        hash: hash_full,
        subject,
        body,
        author: fields[1].to_string(),
        author_email: fields[2].to_string(),
        author_time_secs: fields[3].parse().unwrap_or(0),
        committer: fields[4].to_string(),
        committer_email: fields[5].to_string(),
        committer_time_secs: fields[6].parse().unwrap_or(0),
        parents,
        files,
    })
}

/// 提交中单个文件的差异（对比第一个父提交；根提交对比空树），返回 DiffRow 行
fn git_commit_file_diff(root: &str, hash: &str, path: &str) -> Result<Vec<DiffRow>, GitError> {
    let path = validate_rel_path(path)?;
    let hash = hash.trim();
    if hash.is_empty() {
        return Err(git_err("提交哈希不能为空"));
    }
    let git_bin = git_bin()?;
    git_rev_parse_with(&git_bin, root)?;
    let parents = commit_parents(&git_bin, root, hash)?;
    let base = commit_base_from_parents(&parents);
    // 文本判定（与资源面板 session_fs_probe_text 同一规则）：新旧两个版本
    // 中任一侧为二进制都报错；某一侧不存在（新增/删除）时只探测存在的一侧，
    // 两侧都取不到时继续执行 diff（该路径在两树中都不存在时行集为空）。
    let new_is_text = blob_bytes(&git_bin, root, &hash, &path)
        .as_deref()
        .map(content_is_text)
        .unwrap_or(true);
    let old_is_text = blob_bytes(&git_bin, root, &base, &path)
        .as_deref()
        .map(content_is_text)
        .unwrap_or(true);
    if !new_is_text || !old_is_text {
        return Err(git_err("该文件是二进制文件，无法显示文本差异"));
    }
    let (code, out, stderr) = git_output(
        &git_bin,
        root,
        &[
            "diff",
            "-M",
            "--text",
            "--no-ext-diff",
            "--unified=3",
            &base,
            hash,
            "--",
            &path,
        ],
    )?;
    if code != 0 {
        return Err(git_err(format!("git diff 失败: {}", stderr.trim())));
    }
    if out.len() as u64 > MAX_FILE_BYTES {
        return Err(git_err(format!(
            "文件过大（超过 {} MB），无法显示差异: {path}",
            MAX_FILE_BYTES / (1024 * 1024)
        )));
    }
    crate::codex::diff::build_commit_diff_rows(&out).map_err(git_err)
}

/// 内容文本判定采样字节数（与 session_fs 文本探测窗口一致）
const TEXT_PROBE_BYTES: usize = 8000;

/// 应用文本判定：前 8000 字节无 NUL 且合法 UTF-8（采样边界截断容忍）→ 文本。
/// 与资源面板 session_fs_probe_text 共用同一规则，保证两处对同一文件判定一致。
fn content_is_text(data: &[u8]) -> bool {
    looks_text(&data[..data.len().min(TEXT_PROBE_BYTES)])
}

/// 读取仓库内 blob 原始字节（`git cat-file blob <rev>:<path>`）；不存在返回 None
fn blob_bytes(git_bin: &Path, root: &str, rev: &str, path: &str) -> Option<Vec<u8>> {
    let (code, out, _) = git_output_raw(
        git_bin,
        root,
        &["cat-file", "blob", &format!("{rev}:{path}")],
    )
    .ok()?;
    if code != 0 {
        return None;
    }
    Some(out)
}

/// 探测工作区文件是否为文本：读取前 8000 字节按应用规则判定；
/// 文件不存在或不可读时返回 None（由调用方回退到 HEAD 版本探测）。
fn worktree_file_is_text(root: &Path, path: &str) -> Option<bool> {
    let mut buf = vec![0u8; TEXT_PROBE_BYTES];
    let mut file = std::fs::File::open(root.join(path)).ok()?;
    let mut n = 0;
    while n < buf.len() {
        match file.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(read) => n += read,
            Err(_) => return None,
        }
    }
    Some(content_is_text(&buf[..n]))
}

/// diff 基准统一为 HEAD（暂存 + 未暂存合并）；未跟踪文件与空内容对比。
/// 返回空串表示无内容变化；二进制返回错误提示。
fn git_diff(root: &str, path: &str, _kind: &str) -> Result<String, GitError> {
    let path = validate_rel_path(path)?;
    let git_bin = git_bin()?;
    let repo = git_rev_parse_with(&git_bin, root)?;
    let in_head = git_output(&git_bin, root, &["cat-file", "-e", &format!("HEAD:{path}")])?.0 == 0;
    // 文本判定（与资源面板同一规则）：优先探测工作区文件，删除/不可读时回退
    // HEAD blob；两者都不可判定时继续执行 diff，由 git 输出兜底。
    let is_text = match worktree_file_is_text(&repo.workdir, &path) {
        Some(text) => text,
        None => blob_bytes(&git_bin, root, "HEAD", &path)
            .as_deref()
            .map(content_is_text)
            .unwrap_or(true),
    };
    if !is_text {
        return Err(git_err("该文件是二进制文件，无法显示文本差异"));
    }
    let (code, out, stderr) = if in_head {
        git_output(
            &git_bin,
            root,
            &[
                "diff",
                "--text",
                "--no-ext-diff",
                "--unified=3",
                "HEAD",
                "--",
                &path,
            ],
        )?
    } else {
        // 未跟踪/新增：与空内容对比（`git diff --no-index` 有差异时退出码为 1）
        let abs = repo.workdir.join(&path);
        git_output(
            &git_bin,
            root,
            &[
                "diff",
                "--text",
                "--no-index",
                "--",
                "/dev/null",
                abs.to_string_lossy().as_ref(),
            ],
        )?
    };
    if code != 0 && code != 1 {
        return Err(git_err(format!("git diff 失败: {}", stderr.trim())));
    }
    if out.len() as u64 > MAX_FILE_BYTES {
        return Err(git_err(format!(
            "文件过大（超过 {} MB），无法显示差异: {path}",
            MAX_FILE_BYTES / (1024 * 1024)
        )));
    }
    if out.trim().is_empty() {
        return Ok(String::new());
    }
    Ok(out)
}

// ---------- 拉取与推送 ----------

/// 拉取：调用系统 git（快进优先，`--ff-only`）。分叉时不自动合并，
/// 报错提示先经 GitView 分支合并手动合并，绝不留下冲突状态。
fn git_pull(root: &str) -> Result<GitPullResult, GitError> {
    let git_bin = git_bin()?;
    git_pull_with(&git_bin, root)
}

/// pull 实现（git 可执行文件路径可注入，便于测试“git 缺失”分支）
fn git_pull_with(git_bin: &Path, root: &str) -> Result<GitPullResult, GitError> {
    git_rev_parse_with(git_bin, root)?;
    let branch = git_current_branch_with(git_bin, root)?;
    if branch == "HEAD" {
        return Err(git_err("游离 HEAD 状态无法拉取，请先切换到某个分支"));
    }
    let (remote_label, remote_branch) = git_remote_target_with(git_bin, root, &branch)?;
    let (code, stdout, stderr) = git_output(
        git_bin,
        root,
        &["pull", "--no-rebase", "--ff-only", &remote_label, &remote_branch],
    )?;
    let combined = format!("{stdout}\n{stderr}");
    if code != 0 {
        return Err(git_err(pull_failure_message(&combined)));
    }
    let status = git_status_with(git_bin, root)?;
    let (kind, message) = if combined.contains("Already up to date") {
        ("up_to_date".to_string(), "本地已是最新".to_string())
    } else {
        (
            "fast_forward".to_string(),
            format!("已快进更新到远端 {remote_label}/{remote_branch}"),
        )
    };
    Ok(GitPullResult { status, kind, message })
}

/// 把 git pull 失败输出映射为友好中文提示（LANG=C 下输出为英文，关键词稳定）
fn pull_failure_message(combined: &str) -> String {
    let c = combined.to_lowercase();
    if c.contains("not possible to fast-forward") {
        "本地与远端已分叉，无法快进拉取，请先在 GitView 中合并分支后再拉取".to_string()
    } else if c.contains("would be overwritten by merge")
        || c.contains("untracked working tree files would be overwritten")
        || c.contains("local changes")
    {
        "本地更改会被拉取覆盖，请先提交或还原后再拉取".to_string()
    } else if c.contains("authentication failed")
        || c.contains("could not read username")
        || c.contains("terminal prompts disabled")
        || c.contains("401")
        || c.contains("403")
    {
        "拉取认证失败，请检查 Git 凭证（如 Git Credential Manager）".to_string()
    } else if c.contains("couldn't find remote ref") || c.contains("no such branch") {
        "远端没有该分支，无法拉取".to_string()
    } else {
        let trimmed = combined.trim();
        if trimmed.is_empty() {
            "git pull 失败".to_string()
        } else {
            trimmed.to_string()
        }
    }
}

/// 推送当前分支到上游远端（调用系统 git；首次推送自动设置上游 -u）。
fn git_push(root: &str) -> Result<GitPushResult, GitError> {
    let git_bin = git_bin()?;
    git_push_with(&git_bin, root)
}

/// push 实现（git 可执行文件路径可注入，便于测试“git 缺失”分支）
fn git_push_with(git_bin: &Path, root: &str) -> Result<GitPushResult, GitError> {
    git_rev_parse_with(git_bin, root)?;
    let branch = git_current_branch_with(git_bin, root)?;
    if branch == "HEAD" {
        return Err(git_err("游离 HEAD 状态无法推送，请先切换到某个分支"));
    }
    let (remote_label, remote_branch) = git_remote_target_with(git_bin, root, &branch)?;
    let refspec = format!("{branch}:{remote_branch}");
    let (code, stdout, stderr) =
        git_output(git_bin, root, &["push", "-u", &remote_label, &refspec])?;
    let combined = format!("{stdout}\n{stderr}");
    if code != 0 {
        return Err(git_err(push_failure_message(&combined)));
    }
    let status = git_status_with(git_bin, root)?;
    let kind = if combined.contains("Everything up-to-date") {
        "up_to_date"
    } else {
        "pushed"
    };
    let message = if kind == "up_to_date" {
        format!("本地已是最新（{remote_label}/{remote_branch}）")
    } else {
        format!("已推送到 {remote_label}/{remote_branch}")
    };
    Ok(GitPushResult {
        status,
        kind: kind.to_string(),
        message,
    })
}

/// 把 git push 失败输出映射为友好中文提示（LANG=C 下输出为英文，关键词稳定）
fn push_failure_message(combined: &str) -> String {
    let c = combined.to_lowercase();
    if c.contains("fetch first") || c.contains("non-fast-forward") || c.contains("[rejected]") {
        "推送被拒绝：远端领先本地，请先拉取合并后再推送".to_string()
    } else if c.contains("authentication failed")
        || c.contains("could not read username")
        || c.contains("terminal prompts disabled")
        || c.contains("401")
        || c.contains("403")
    {
        "推送认证失败，请检查 Git 凭证（如 Git Credential Manager）".to_string()
    } else if c.contains("does not match any") || c.contains("src refspec") {
        "当前分支还没有提交，无法推送".to_string()
    } else {
        let trimmed = combined.trim();
        if trimmed.is_empty() {
            "git push 失败".to_string()
        } else {
            trimmed.to_string()
        }
    }
}

/// 解析远端目标（远端名 + 远端分支名）：优先已配置的上游，
/// 无配置时以默认远端（origin）同名分支兜底。
fn git_remote_target_with(
    git_bin: &Path,
    root: &str,
    branch: &str,
) -> Result<(String, String), GitError> {
    let remote = git_config_with(git_bin, root, &format!("branch.{branch}.remote"))?;
    let merge = git_config_with(git_bin, root, &format!("branch.{branch}.merge"))?;
    if let (Some(r), Some(m)) = (remote, merge) {
        let remote_branch = m.strip_prefix("refs/heads/").unwrap_or(&m).to_string();
        return Ok((r, remote_branch));
    }
    let names = git_remote_names_with(git_bin, root)?;
    let label = if names.iter().any(|n| n == "origin") {
        "origin".to_string()
    } else if let Some(first) = names.first() {
        first.clone()
    } else {
        return Err(git_err("未找到可用的远端"));
    };
    Ok((label, branch.to_string()))
}

/// 读取单个 git config 键（缺失时返回 None）
fn git_config_with(git_bin: &Path, root: &str, key: &str) -> Result<Option<String>, GitError> {
    let (code, stdout, _) = git_output(git_bin, root, &["config", "--get", key])?;
    if code != 0 {
        return Ok(None);
    }
    let v = stdout.trim().to_string();
    if v.is_empty() {
        Ok(None)
    } else {
        Ok(Some(v))
    }
}

/// 远端名列表（`git remote`）
fn git_remote_names_with(git_bin: &Path, root: &str) -> Result<Vec<String>, GitError> {
    let (code, stdout, _) = git_output(git_bin, root, &["remote"])?;
    if code != 0 {
        return Err(git_err("读取远端列表失败"));
    }
    Ok(stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

// ---------- 远端管理 ----------

/// 远端列表（`git remote -v`）：fetch/push 地址；pushUrl 未配置时回退为拉取地址
fn git_remote(root: &str) -> Result<GitRemotes, GitError> {
    let git_bin = git_bin()?;
    git_remote_with(&git_bin, root)
}

fn git_remote_with(git_bin: &Path, root: &str) -> Result<GitRemotes, GitError> {
    git_rev_parse_with(git_bin, root)?;
    let (code, stdout, stderr) = git_output(git_bin, root, &["remote", "-v"])?;
    if code != 0 {
        return Err(git_err(format!("读取远端列表失败: {}", stderr.trim())));
    }
    let mut remotes: Vec<GitRemote> = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        let (name, rest) = match line.split_once('\t') {
            Some((n, r)) => (n, r.trim()),
            None => continue,
        };
        let (url, is_push) = if let Some(u) = rest.strip_suffix(" (fetch)") {
            (u.trim(), false)
        } else if let Some(u) = rest.strip_suffix(" (push)") {
            (u.trim(), true)
        } else {
            continue;
        };
        let url = url.to_string();
        if let Some(existing) = remotes.iter_mut().find(|r| r.name == name) {
            if is_push {
                existing.push_url = Some(url);
            } else {
                existing.fetch_url = Some(url);
            }
        } else {
            remotes.push(GitRemote {
                name: name.to_string(),
                fetch_url: if is_push { None } else { Some(url.clone()) },
                push_url: if is_push { Some(url) } else { None },
            });
        }
    }
    remotes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let current = git_current_branch_with(git_bin, root)?;
    let current = if current == "HEAD" {
        None
    } else {
        git_remote_target_with(git_bin, root, &current)
            .ok()
            .map(|(label, _)| label)
    };
    Ok(GitRemotes { current, remotes })
}

/// 远端名合法性校验（git remote 子命令规则的子集，其余由 git 自身兜底校验）
fn validate_remote_name(name: &str) -> Result<(), GitError> {
    if name.is_empty() || name.trim() != name {
        return Err(git_err("远端名不能为空或包含首尾空格"));
    }
    if name.starts_with('-')
        || name.contains('/')
        || name.contains("..")
        || name.ends_with('.')
        || name.contains(['~', '^', ':', '?', '*', '[', '\\'])
        || name.contains(' ')
        || name.contains("@{")
    {
        return Err(git_err("远端名不合法（不能以 - 开头、包含 /、.. 或空格，也不能以 . 结尾）"));
    }
    Ok(())
}

/// 把 git remote 失败输出映射为友好中文提示（LANG=C 下输出为英文，关键词稳定）
fn remote_failure_message(combined: &str) -> String {
    let c = combined.to_lowercase();
    if c.contains("already exists") {
        "远端已存在".to_string()
    } else if c.contains("no such remote") {
        "远端不存在".to_string()
    } else {
        let trimmed = combined.trim();
        if trimmed.is_empty() {
            "git remote 操作失败".to_string()
        } else {
            trimmed.to_string()
        }
    }
}

/// 添加远端（`git remote add <name> <url>`，自动生成默认 fetch refspec）
fn git_remote_add(root: &str, name: &str, url: &str) -> Result<GitRemotes, GitError> {
    let git_bin = git_bin()?;
    git_remote_add_with(&git_bin, root, name, url)
}

fn git_remote_add_with(
    git_bin: &Path,
    root: &str,
    name: &str,
    url: &str,
) -> Result<GitRemotes, GitError> {
    let name = name.trim();
    let url = url.trim();
    validate_remote_name(name)?;
    if url.is_empty() {
        return Err(git_err("远端地址不能为空"));
    }
    git_run_mapped(git_bin, root, &["remote", "add", name, url], remote_failure_message)?;
    git_remote_with(git_bin, root)
}

/// 修改远端拉取地址（`git remote set-url <name> <url>`；pushUrl 保持不变）
fn git_remote_set_url(root: &str, name: &str, url: &str) -> Result<GitRemotes, GitError> {
    let git_bin = git_bin()?;
    git_remote_set_url_with(&git_bin, root, name, url)
}

fn git_remote_set_url_with(
    git_bin: &Path,
    root: &str,
    name: &str,
    url: &str,
) -> Result<GitRemotes, GitError> {
    let name = name.trim();
    let url = url.trim();
    validate_remote_name(name)?;
    if url.is_empty() {
        return Err(git_err("远端地址不能为空"));
    }
    git_run_mapped(git_bin, root, &["remote", "set-url", name, url], remote_failure_message)?;
    git_remote_with(git_bin, root)
}

/// 删除远端（`git remote remove <name>`；删除当前上游远端允许，由前端确认提示）
fn git_remote_remove(root: &str, name: &str) -> Result<GitRemotes, GitError> {
    let git_bin = git_bin()?;
    git_remote_remove_with(&git_bin, root, name)
}

fn git_remote_remove_with(git_bin: &Path, root: &str, name: &str) -> Result<GitRemotes, GitError> {
    let name = name.trim();
    validate_remote_name(name)?;
    git_run_mapped(git_bin, root, &["remote", "remove", name], remote_failure_message)?;
    git_remote_with(git_bin, root)
}

// ---------- 远程分支管理 ----------

/// 解析「远端/分支」→ (远端名, 分支名)
fn split_remote_branch(remote_branch: &str) -> Result<(&str, &str), GitError> {
    let rb = remote_branch.trim();
    let idx = rb
        .find('/')
        .ok_or_else(|| git_err("远程分支名格式应为 远端/分支"))?;
    let (remote, branch) = (&rb[..idx], &rb[idx + 1..]);
    if remote.is_empty() || branch.is_empty() {
        return Err(git_err("远程分支名格式应为 远端/分支"));
    }
    Ok((remote, branch))
}

/// 把 git fetch 失败输出映射为友好中文提示（LANG=C 下输出为英文，关键词稳定）
fn fetch_failure_message(combined: &str) -> String {
    let c = combined.to_lowercase();
    if c.contains("authentication failed")
        || c.contains("could not read username")
        || c.contains("terminal prompts disabled")
        || c.contains("401")
        || c.contains("403")
    {
        "拉取认证失败，请检查 Git 凭证（如 Git Credential Manager）".to_string()
    } else if c.contains("couldn't find remote ref") || c.contains("no such branch") {
        "远端没有该分支，无法拉取".to_string()
    } else {
        let trimmed = combined.trim();
        if trimmed.is_empty() {
            "git fetch 失败".to_string()
        } else {
            trimmed.to_string()
        }
    }
}

/// 把 git checkout / 删除远程分支失败输出映射为友好中文提示
fn remote_branch_failure_message(combined: &str) -> String {
    let c = combined.to_lowercase();
    if c.contains("cannot be created from it")
        || c.contains("did not match")
        || c.contains("pathspec")
    {
        "远端分支不存在，请先拉取刷新".to_string()
    } else if c.contains("remote ref does not exist") || c.contains("unable to delete") {
        "远端分支不存在，无法删除".to_string()
    } else if c.contains("authentication failed")
        || c.contains("could not read username")
        || c.contains("terminal prompts disabled")
        || c.contains("401")
        || c.contains("403")
    {
        "操作认证失败，请检查 Git 凭证（如 Git Credential Manager）".to_string()
    } else {
        let trimmed = combined.trim();
        if trimmed.is_empty() {
            "git 远程分支操作失败".to_string()
        } else {
            trimmed.to_string()
        }
    }
}

/// 拉取远端更新（`git fetch <remote>`；remote 为空时取默认远端），返回刷新后的分支列表
/// 拉取远端更新（`git fetch <remote>`；remote 为空时取默认远端），返回刷新后的分支列表
fn git_fetch(root: &str, remote: &str) -> Result<GitBranches, GitError> {
    let git_bin = git_bin()?;
    git_fetch_with(&git_bin, root, remote)
}

fn git_fetch_with(git_bin: &Path, root: &str, remote: &str) -> Result<GitBranches, GitError> {
    git_rev_parse_with(git_bin, root)?;
    let trimmed = remote.trim();
    let remote_label = if trimmed.is_empty() {
        let names = git_remote_names_with(git_bin, root)?;
        if names.iter().any(|n| n == "origin") {
            "origin".to_string()
        } else if let Some(first) = names.first() {
            first.clone()
        } else {
            return Err(git_err("未找到可用的远端"));
        }
    } else {
        trimmed.to_string()
    };
    git_run_mapped(git_bin, root, &["fetch", &remote_label], fetch_failure_message)?;
    git_branch_with(git_bin, root)
}

/// 从远程分支检出为同名本地跟踪分支（`git checkout -b <branch> --track <remote>/<branch>`）。
/// 本地已有同名分支时拒绝（由前端路由到本地切换）。
fn git_checkout(root: &str, remote_branch: &str) -> Result<GitStatus, GitError> {
    let git_bin = git_bin()?;
    git_checkout_with(&git_bin, root, remote_branch)
}

fn git_checkout_with(
    git_bin: &Path,
    root: &str,
    remote_branch: &str,
) -> Result<GitStatus, GitError> {
    let (remote_label, branch) = split_remote_branch(remote_branch)?;
    validate_remote_name(remote_label)?;
    validate_branch_name(branch)?;
    git_rev_parse_with(git_bin, root)?;
    if git_ref_exists_with(git_bin, root, &format!("refs/heads/{branch}"))? {
        return Err(git_err(format!("本地分支 {branch} 已存在，请先切换到该分支")));
    }
    let track = format!("{remote_label}/{branch}");
    git_run_mapped(
        git_bin,
        root,
        &["checkout", "-b", branch, "--track", &track],
        remote_branch_failure_message,
    )?;
    git_status_with(git_bin, root)
}

/// 删除远程分支（`git push <remote> --delete <branch>`），返回刷新后的分支列表
fn git_push_delete(root: &str, remote_branch: &str) -> Result<GitBranches, GitError> {
    let git_bin = git_bin()?;
    git_push_delete_with(&git_bin, root, remote_branch)
}

fn git_push_delete_with(
    git_bin: &Path,
    root: &str,
    remote_branch: &str,
) -> Result<GitBranches, GitError> {
    let (remote_label, branch) = split_remote_branch(remote_branch)?;
    validate_remote_name(remote_label)?;
    validate_branch_name(branch)?;
    git_rev_parse_with(git_bin, root)?;
    git_run_mapped(
        git_bin,
        root,
        &["push", remote_label, "--delete", branch],
        remote_branch_failure_message,
    )?;
    git_branch_with(git_bin, root)
}

/// 把 `git branch --set-upstream-to` 失败输出映射为友好中文提示
fn switch_upstream_failure_message(combined: &str) -> String {
    let c = combined.to_lowercase();
    if c.contains("does not exist") || c.contains("not a valid branch") {
        "远端没有同名分支，请先拉取刷新".to_string()
    } else {
        let trimmed = combined.trim();
        if trimmed.is_empty() {
            "设置上游失败".to_string()
        } else {
            trimmed.to_string()
        }
    }
}

/// 把当前分支上游切换到 `remote` 的同名分支（`git branch --set-upstream-to <remote>/<分支>`，
/// 单上游替换）。返回刷新后的远端列表。
fn git_set_upstream(root: &str, remote: &str) -> Result<GitRemotes, GitError> {
    let git_bin = git_bin()?;
    git_set_upstream_with(&git_bin, root, remote)
}

fn git_set_upstream_with(
    git_bin: &Path,
    root: &str,
    remote: &str,
) -> Result<GitRemotes, GitError> {
    let remote = remote.trim();
    validate_remote_name(remote)?;
    git_rev_parse_with(git_bin, root)?;
    let (code, _, _) = git_output(git_bin, root, &["remote", "get-url", remote])?;
    if code != 0 {
        return Err(git_err(format!("远端 {remote} 不存在")));
    }
    let branch = git_current_branch_with(git_bin, root)?;
    if branch == "HEAD" {
        return Err(git_err("游离 HEAD 状态无法设置上游，请先切换到某个分支"));
    }
    let target = format!("{remote}/{branch}");
    git_run_mapped(
        git_bin,
        root,
        &["branch", "--set-upstream-to", &target],
        switch_upstream_failure_message,
    )?;
    git_remote_with(git_bin, root)
}

// ---------- 分支合并 ----------

/// 合并本地分支到当前分支（`git merge <name>`，标准语义）：
/// 快进或三路合并（git 自动创建合并提交）；冲突一律中止并 `git merge --abort`，
/// 绝不留下冲突文件；本地更改会被合并覆盖时在合并前直接拒绝。
fn git_merge(root: &str, name: &str) -> Result<GitMergeResult, GitError> {
    let name = name.trim();
    let git_bin = git_bin()?;
    git_rev_parse_with(&git_bin, root)?;
    let branch = git_current_branch_with(&git_bin, root)?;
    if branch == "HEAD" {
        return Err(git_err("游离 HEAD 状态无法合并，请先切换到某个分支"));
    }
    if branch == name {
        return Err(git_err("不能将当前分支合并到自身"));
    }
    if !git_ref_exists_with(&git_bin, root, &format!("refs/heads/{name}"))? {
        return Err(git_err(format!("分支 {name} 不存在")));
    }
    if !git_head_exists_with(&git_bin, root)? {
        return Err(git_err("仓库还没有任何提交，无法合并"));
    }
    let source_tip = git_rev_parse_ref_with(&git_bin, root, &format!("refs/heads/{name}"))?;
    let (code, stdout, stderr) = git_output(&git_bin, root, &["merge", name])?;
    let combined = format!("{stdout}\n{stderr}");
    if code != 0 {
        let c = combined.to_lowercase();
        // 冲突 → 立即中止，不留冲突文件
        if c.contains("conflict") {
            let _ = git_output(&git_bin, root, &["merge", "--abort"]);
            return Err(git_err(format!(
                "合并 {name} 存在冲突，已中止（未修改任何文件）。冲突文件：{}",
                extract_conflict_paths(&combined)
            )));
        }
        // 本地更改会被合并覆盖 → git 在合并前拒绝，无状态残留
        if c.contains("would be overwritten by merge") {
            return Err(git_err(format!(
                "本地更改会被合并覆盖，请先提交或还原后再合并：{}",
                extract_listed_paths(&combined)
            )));
        }
        let trimmed = combined.trim();
        return Err(git_err(if trimmed.is_empty() {
            format!("合并分支 {name} 失败")
        } else {
            trimmed.to_string()
        }));
    }
    let head_after = git_rev_parse_head_with(&git_bin, root)?;
    let status = git_status_with(&git_bin, root)?;
    let (kind, message) = if combined.to_lowercase().contains("already up to date") {
        ("up_to_date".to_string(), "分支已是最新，无需合并".to_string())
    } else if head_after == source_tip {
        (
            "fast_forward".to_string(),
            format!("已将分支 {name} 快进合并到 {branch}"),
        )
    } else {
        (
            "merged".to_string(),
            format!("已将分支 {name} 合并到 {branch}"),
        )
    };
    Ok(GitMergeResult { status, kind, message })
}

/// 取当前 HEAD 的完整 hash
fn git_rev_parse_head_with(git_bin: &Path, root: &str) -> Result<String, GitError> {
    git_rev_parse_ref_with(git_bin, root, "HEAD")
}

/// 取指定引用指向的完整 hash
fn git_rev_parse_ref_with(git_bin: &Path, root: &str, reference: &str) -> Result<String, GitError> {
    let (code, stdout, stderr) = git_output(git_bin, root, &["rev-parse", reference])?;
    if code != 0 {
        return Err(git_err(format!("解析 {reference} 失败: {}", stderr.trim())));
    }
    Ok(stdout.trim().to_string())
}

/// 从 git merge 输出提取冲突文件列表（`CONFLICT ...: Merge conflict in <path>`）
fn extract_conflict_paths(combined: &str) -> String {
    let mut paths: Vec<String> = Vec::new();
    for line in combined.lines() {
        let l = line.trim();
        if let Some(idx) = l.find("conflict in ") {
            let p = l[idx + "conflict in ".len()..].trim();
            let p = p.trim_end_matches(':').trim();
            if !p.is_empty() && !paths.contains(&p.to_string()) {
                paths.push(p.to_string());
            }
        }
    }
    if paths.is_empty() {
        "<未知路径>".to_string()
    } else {
        paths.join("、")
    }
}

/// 在阻塞线程中执行同步 git 操作，带指定超时（秒）；错误文案与旧实现保持一致。
async fn run_blocking_with_timeout<T: Send + 'static>(
    timeout_secs: u64,
    f: impl FnOnce() -> Result<T, GitError> + Send + 'static,
) -> Result<T, GitError> {
    let task_f = move || {
        // 锁在任务内部持有，超时后任务仍在后台执行时，后续 git 操作会排队等待，
        // 避免与未结束的操作并发读写同一仓库。
        let _guard = git_op_lock().lock().unwrap_or_else(|e| e.into_inner());
        f()
    };
    match spawn_blocking_timeout(timeout_secs, task_f).await {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => Err(e),
        Err(BlockingError::Timeout(secs)) => Err(git_err(format!("git 操作超时（{secs} 秒）"))),
        Err(BlockingError::Join(e)) => Err(git_err(format!("git 任务异常: {e}"))),
    }
}

/// 在阻塞线程中执行同步 git 操作，带 60s 超时
async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, GitError> + Send + 'static,
) -> Result<T, GitError> {
    run_blocking_with_timeout(60, f).await
}

#[tauri::command]
pub async fn git_changes_status(workspace: String) -> Result<GitStatus, GitError> {
    run_blocking(move || git_status(&workspace)).await
}

#[tauri::command]
pub async fn git_changes_commit(workspace: String, message: String) -> Result<GitStatus, GitError> {
    run_blocking(move || git_commit(&workspace, &message)).await
}

#[tauri::command]
pub async fn git_changes_pull(workspace: String) -> Result<GitPullResult, GitError> {
    // 拉取涉及网络传输，放宽超时到 10 分钟
    run_blocking_with_timeout(600, move || git_pull(&workspace)).await
}

#[tauri::command]
pub async fn git_changes_push(workspace: String) -> Result<GitPushResult, GitError> {
    // 推送涉及网络传输与可能的凭证交互，放宽超时到 10 分钟
    run_blocking_with_timeout(600, move || git_push(&workspace)).await
}

/// 远端列表（`git remote -v`）
#[tauri::command]
pub async fn git_changes_remotes(workspace: String) -> Result<GitRemotes, GitError> {
    run_blocking(move || git_remote(&workspace)).await
}

/// 添加远端（调用系统 git）
#[tauri::command]
pub async fn git_changes_remote_add(workspace: String, name: String, url: String) -> Result<GitRemotes, GitError> {
    run_blocking(move || git_remote_add(&workspace, &name, &url)).await
}

/// 修改远端拉取地址（调用系统 git）
#[tauri::command]
pub async fn git_changes_remote_set_url(
    workspace: String,
    name: String,
    url: String,
) -> Result<GitRemotes, GitError> {
    run_blocking(move || git_remote_set_url(&workspace, &name, &url)).await
}

/// 删除远端（调用系统 git）
#[tauri::command]
pub async fn git_changes_remote_remove(workspace: String, name: String) -> Result<GitRemotes, GitError> {
    run_blocking(move || git_remote_remove(&workspace, &name)).await
}

/// 拉取远端更新（remote 为空时取默认远端；网络操作放宽超时到 10 分钟）
#[tauri::command]
pub async fn git_changes_remote_fetch(workspace: String, remote: String) -> Result<GitBranches, GitError> {
    run_blocking_with_timeout(600, move || git_fetch(&workspace, &remote)).await
}

/// 从远程分支检出为同名本地跟踪分支（调用系统 git）
#[tauri::command]
pub async fn git_changes_branch_checkout_remote(
    workspace: String,
    remote_branch: String,
) -> Result<GitStatus, GitError> {
    run_blocking(move || git_checkout(&workspace, &remote_branch)).await
}

/// 删除远程分支（调用系统 git；网络操作放宽超时到 10 分钟）
#[tauri::command]
pub async fn git_changes_remote_branch_delete(
    workspace: String,
    remote_branch: String,
) -> Result<GitBranches, GitError> {
    run_blocking_with_timeout(600, move || git_push_delete(&workspace, &remote_branch)).await
}

/// 把当前分支上游切换到指定远端的同名分支（调用系统 git，单上游替换）
#[tauri::command]
pub async fn git_changes_remote_switch_upstream(
    workspace: String,
    remote: String,
) -> Result<GitRemotes, GitError> {
    run_blocking(move || git_set_upstream(&workspace, &remote)).await
}

/// 本机 git 可用性（返回启动探测的缓存结果）
#[tauri::command]
pub async fn git_changes_git_available(workspace: String) -> bool {
    let _ = workspace;
    git_bin().is_ok()
}

#[tauri::command]
pub async fn git_changes_init(workspace: String) -> Result<GitStatus, GitError> {
    run_blocking(move || git_init(&workspace)).await
}

#[tauri::command]
pub async fn git_changes_branches(workspace: String) -> Result<GitBranches, GitError> {
    run_blocking(move || git_branch(&workspace)).await
}

#[tauri::command]
pub async fn git_changes_branch_create(workspace: String, name: String) -> Result<GitStatus, GitError> {
    run_blocking(move || git_branch_create(&workspace, &name)).await
}

#[tauri::command]
pub async fn git_changes_branch_delete(workspace: String, name: String) -> Result<GitStatus, GitError> {
    run_blocking(move || git_branch_delete(&workspace, &name)).await
}

#[tauri::command]
pub async fn git_changes_branch_switch(workspace: String, name: String) -> Result<GitStatus, GitError> {
    run_blocking(move || git_switch(&workspace, &name)).await
}

#[tauri::command]
pub async fn git_changes_branch_merge(workspace: String, name: String) -> Result<GitMergeResult, GitError> {
    run_blocking(move || git_merge(&workspace, &name)).await
}

#[tauri::command]
pub async fn git_changes_log(
    workspace: String,
    limit: usize,
    before: Option<String>,
) -> Result<Vec<GitCommitEntry>, GitError> {
    run_blocking(move || git_log(&workspace, limit, before)).await
}

#[tauri::command]
pub async fn git_changes_commit_detail(
    workspace: String,
    hash: String,
) -> Result<GitCommitDetail, GitError> {
    run_blocking(move || git_commit_detail(&workspace, &hash)).await
}

#[tauri::command]
pub async fn git_changes_commit_file_diff(
    workspace: String,
    hash: String,
    path: String,
) -> Result<Vec<DiffRow>, GitError> {
    run_blocking(move || git_commit_file_diff(&workspace, &hash, &path)).await
}

#[tauri::command]
pub async fn git_changes_diff(workspace: String, path: String, kind: String) -> Result<String, GitError> {
    run_blocking(move || git_diff(&workspace, &path, &kind)).await
}

#[tauri::command]
pub async fn git_changes_stage(workspace: String, path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || git_add(&workspace, &path)).await
}

#[tauri::command]
pub async fn git_changes_unstage(workspace: String, path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || git_unstage(&workspace, &path)).await
}

#[tauri::command]
pub async fn git_changes_stage_all(workspace: String) -> Result<GitStatus, GitError> {
    run_blocking(move || git_add_all(&workspace)).await
}

#[tauri::command]
pub async fn git_changes_unstage_all(workspace: String) -> Result<GitStatus, GitError> {
    run_blocking(move || git_unstage_all(&workspace)).await
}

#[tauri::command]
pub async fn git_changes_restore(workspace: String, path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || git_restore(&workspace, &path)).await
}

#[tauri::command]
pub async fn git_changes_delete(workspace: String, path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || git_rm(&workspace, &path)).await
}

#[tauri::command]
pub async fn git_changes_ignore(workspace: String, path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || git_ignore(&workspace, &path)).await
}

// ---------- .git / 工作区监听 ----------

/// 事件路径是否位于需要排除的“点目录”下（除 .git 外的 . 开头目录，任意深度）。
/// 组件是否目录用路径前缀 is_dir() 判断：`.gitignore`、`.env` 等点文件不排除。
fn path_under_excluded_dot_dir(root: &Path, p: &Path) -> bool {
    let Some(rel) = shared_rel_path_of(root, p) else {
        return false;
    };
    let mut acc = root.to_path_buf();
    for c in rel.split('/') {
        if c.is_empty() || c == "." || c == ".." {
            continue;
        }
        acc.push(c);
        if c.starts_with('.') && c.len() > 1 && c != ".git" && acc.is_dir() {
            return true;
        }
        if c.eq_ignore_ascii_case("node_modules") && acc.is_dir() {
            return true;
        }
    }
    false
}

/// 忽略快照：`git ls-files -oi --exclude-standard --directory -z` 解析结果，
/// 用于 notify 回调过滤被忽略路径（避免对忽略内容触发 git status 刷新）
#[derive(Default)]
struct IgnoreSnapshot {
    /// 被整体忽略的目录（相对仓库根、正斜杠、小写；不含尾斜杠）
    dirs: HashSet<String>,
    /// 被忽略的精确文件（相对仓库根、正斜杠、小写）
    files: HashSet<String>,
}

/// 归一化忽略匹配键：统一正斜杠 + 小写（Windows 大小写不敏感）
fn ignore_key(p: &str) -> String {
    p.replace('\\', "/").to_lowercase()
}

/// 解析 `git ls-files -z` 输出：尾 `/` 为被整体忽略的目录，其余为精确文件
fn parse_ignored_list(output: &str) -> IgnoreSnapshot {
    let mut snap = IgnoreSnapshot::default();
    for entry in output.split('\0') {
        let trimmed = entry.trim_end_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        let key = ignore_key(trimmed);
        if entry.ends_with('/') {
            snap.dirs.insert(key);
        } else {
            snap.files.insert(key);
        }
    }
    snap
}

impl IgnoreSnapshot {
    /// 路径是否应被忽略：精确命中文件/目录，或任一祖先目录被整体忽略
    fn is_ignored(&self, rel: &str) -> bool {
        let key = ignore_key(rel);
        if self.files.contains(&key) || self.dirs.contains(&key) {
            return true;
        }
        let mut rest = key.as_str();
        while let Some(idx) = rest.rfind('/') {
            rest = &rest[..idx];
            if self.dirs.contains(rest) {
                return true;
            }
        }
        false
    }
}

/// 用系统 git 重建忽略快照（`git ls-files -oi --exclude-standard --directory -z`）；
/// 失败/非零退出返回空快照（安全回退为不过滤）
fn build_ignore_snapshot(git_bin: &Path, root: &str) -> IgnoreSnapshot {
    let (code, stdout, _) = match git_output(
        git_bin,
        root,
        &["ls-files", "-oi", "--exclude-standard", "--directory", "-z"],
    ) {
        Ok(v) => v,
        Err(_) => return IgnoreSnapshot::default(),
    };
    if code != 0 {
        return IgnoreSnapshot::default();
    }
    parse_ignored_list(&stdout)
}

#[tauri::command]
pub async fn git_changes_watch_start(
    app: AppHandle,
    state: State<'_, GitWatcherState>,
    workspace: String,
) -> Result<(), GitError> {
    // 在阻塞线程中解析仓库：工作区根与 git 目录（git.exe）
    let (repo_root, git_dir) = match run_blocking({
        let root = workspace;
        move || -> Result<(PathBuf, PathBuf), GitError> {
            let git_bin = git_bin()?;
            let info = git_rev_parse_with(&git_bin, &root)?;
            Ok((info.workdir, info.git_dir))
        }
    })
    .await
    {
        Ok(r) => r,
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
    // 忽略快照 + 脏标记：notify 回调置脏，防抖任务 ticker 用 git 重建
    let ignore_snapshot = Arc::new(Mutex::new(IgnoreSnapshot::default()));
    let ignore_dirty = Arc::new(AtomicBool::new(true));
    let ignore_snapshot_cb = ignore_snapshot.clone();
    let ignore_dirty_cb = ignore_dirty.clone();
    let exclude_suffix = PathBuf::from(".git").join("info").join("exclude");
    let exclude_suffix_cb = exclude_suffix.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            for p in ev.paths {
                // .gitignore（任意层级）或 .git/info/exclude 变更：标记重建忽略快照
                let fname = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if fname == ".gitignore" || (fname == "exclude" && p.ends_with(&exclude_suffix_cb))
                {
                    ignore_dirty_cb.store(true, Ordering::SeqCst);
                }
                // 本地廉价过滤：排除 `. 开头目录`（.git 除外）与 node_modules
                if path_under_excluded_dot_dir(&root_for_filter, &p) {
                    continue;
                }
                // 忽略快照过滤：命中 gitignore 的路径不再触发刷新（真实变更仍由
                // 防抖后的 git status 原生过滤）
                if let Some(rel) = shared_rel_path_of(&root_for_filter, &p) {
                    if ignore_snapshot_cb
                        .lock()
                        .map(|s| s.is_ignored(&rel))
                        .unwrap_or(false)
                    {
                        continue;
                    }
                }
                let _ = tx_watcher.try_send(p.clone());
            }
        }
    })
    .map_err(|e| git_err(format!("创建 git 监听失败: {e}")))?;

    watcher
        .watch(&repo_root, RecursiveMode::Recursive)
        .map_err(|e| git_err(format!("监听仓库目录失败 {}: {e}", repo_root.display())))?;

    // git 目录位于工作区之外（linked worktree / 子模块）时追加监听
    if !git_dir.starts_with(&repo_root) && git_dir.is_dir() {
        watcher
            .watch(&git_dir, RecursiveMode::Recursive)
            .map_err(|e| git_err(format!("监听 git 目录失败 {}: {e}", git_dir.display())))?;
    }

    // 防抖任务：最后一次 watch 文件变化静默 300ms 后 emit 变更事件；
    // 顺带在忽略快照脏时用 git 重建（spawn_blocking 内 spawn git 子进程）
    let handle = app.clone();
    let root_for_task = repo_root.clone();
    let ignore_snapshot_task = ignore_snapshot.clone();
    let ignore_dirty_task = ignore_dirty.clone();
    tauri::async_runtime::spawn(async move {
        let mut last_event: Option<tokio::time::Instant> = None;
        let mut ticker = tokio::time::interval(Duration::from_millis(150));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(_) => {
                        last_event = Some(tokio::time::Instant::now());
                    }
                    None => break,
                },
                _ = ticker.tick() => {
                    if ignore_dirty_task.swap(false, Ordering::SeqCst) {
                        let root_for_snap = root_for_task
                            .to_str()
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        let snap_handle = ignore_snapshot_task.clone();
                        if let Ok(snap) = tokio::task::spawn_blocking(move || {
                            match git_bin() {
                                Ok(b) => build_ignore_snapshot(b, &root_for_snap),
                                Err(_) => IgnoreSnapshot::default(),
                            }
                        })
                        .await
                        {
                            if let Ok(mut guard) = snap_handle.lock() {
                                *guard = snap;
                            }
                        }
                    }
                    if let Some(t) = last_event {
                        if t.elapsed() >= Duration::from_millis(WATCH_DEBOUNCE_MS) {
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

    /// 测试夹具：系统 git 仅用于搭建仓库（生产代码不依赖 git）
    fn git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn git(dir: &Path, cmd: &[&str]) -> (i32, String, String) {
        let mut all: Vec<String> = vec!["-C".into(), dir.to_string_lossy().into_owned()];
        all.extend(cmd.iter().map(|s| s.to_string()));
        let out = std::process::Command::new("git")
            .args(&all)
            .output()
            .expect("git 执行失败");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

fn init_committed_repo(dir: &Path) {
    // -b main：与 git init 的默认分支保持一致
    assert_eq!(git(dir, &["init", "-b", "main"]).0, 0);
    assert_eq!(git(dir, &["config", "user.name", "t"]).0, 0);
    assert_eq!(git(dir, &["config", "user.email", "t@t"]).0, 0);
    // 关闭 autocrlf：保证检出/合并/还原写出的内容与提交字节一致（LF）
    assert_eq!(git(dir, &["config", "core.autocrlf", "false"]).0, 0);
    assert_eq!(git(dir, &["add", "-A"]).0, 0);
    assert_eq!(git(dir, &["commit", "-m", "init"]).0, 0);
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
        let st = git_status(root.to_str().unwrap()).unwrap();
        assert!(st.files.is_empty());
        assert!(!st.repo_workspace.is_empty());
        // 对外绝对路径统一反斜杠（git rev-parse 输出正斜杠，clean_path 负责转换）
        assert!(
            !st.repo_workspace.contains('/'),
            "repo_workspace 应使用反斜杠: {}",
            st.repo_workspace
        );
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
        let st = git_status(root.to_str().unwrap()).unwrap();
        let mut by_path: Vec<(String, FileStatus)> =
            st.files.iter().map(|f| (f.path.clone(), f.status)).collect();
        by_path.sort();
        assert_eq!(
            by_path,
            vec![
                ("a.txt".to_string(), FileStatus::Modified),
                ("b.txt".to_string(), FileStatus::Deleted),
                ("c.txt".to_string(), FileStatus::Untracked),
            ]
        );
        assert!(st.files.iter().all(|f| !f.staged && f.worktree));
    }

    #[test]
    fn status_merges_staged_and_worktree_for_same_path() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one").unwrap();
        init_committed_repo(root);
        // 暂存新内容后再次修改工作区 → 同一路径应只出现一条，且优先 added/modified 合并
        std::fs::write(root.join("a.txt"), "two").unwrap();
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);
        std::fs::write(root.join("a.txt"), "three").unwrap();
        let st = git_status(root.to_str().unwrap()).unwrap();
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].path, "a.txt");
        assert_eq!(st.files[0].status, FileStatus::Modified);
        assert!(st.files[0].staged);
        assert!(st.files[0].worktree);
    }

    #[test]
    fn status_flags_staged_and_worktree_independently() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        std::fs::write(root.join("b.txt"), "x\n").unwrap();
        init_committed_repo(root);
        // b.txt：仅工作区修改
        std::fs::write(root.join("b.txt"), "y\n").unwrap();
        // a.txt：先暂存再修改 → 同一条记录同时有 staged 与 worktree
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);
        std::fs::write(root.join("a.txt"), "three\n").unwrap();
        std::fs::write(root.join("c.txt"), "new\n").unwrap();

        let st = git_status(root.to_str().unwrap()).unwrap();
        let a = st.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert!(a.staged && a.worktree);
        let b = st.files.iter().find(|f| f.path == "b.txt").unwrap();
        assert!(!b.staged && b.worktree);
        let c = st.files.iter().find(|f| f.path == "c.txt").unwrap();
        assert!(!c.staged && c.worktree);
        assert_eq!(st.files.len(), 3);
    }

    #[test]
    fn status_respects_gitignore() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        std::fs::write(root.join(".gitignore"), "ignored.log\n").unwrap();
        std::fs::write(root.join("ignored.log"), "x").unwrap();
        std::fs::write(root.join("kept.txt"), "y").unwrap();
        let st = git_status(root.to_str().unwrap()).unwrap();
        let paths: Vec<&str> = st.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"kept.txt"));
        assert!(!paths.contains(&"ignored.log"));
        assert!(paths.contains(&".gitignore"));
    }

    #[test]
    fn status_ignores_node_modules_unconditionally() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::write(root.join("node_modules").join("tracked.txt"), "v1").unwrap();
        init_committed_repo(root);
        // 未 gitignore 的未跟踪文件 + 已被跟踪文件的修改，都无条件隐藏
        std::fs::write(root.join("node_modules").join("new.js"), "x").unwrap();
        std::fs::write(root.join("node_modules").join("tracked.txt"), "v2").unwrap();
        std::fs::write(root.join("a.txt"), "hello2").unwrap();
        let st = git_status(root.to_str().unwrap()).unwrap();
        let paths: Vec<&str> = st.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["a.txt"]);
    }

    #[test]
    fn status_parses_paths_with_spaces_and_special_chars() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        std::fs::write(root.join("my file #1.txt"), "x\n").unwrap();
        std::fs::write(root.join("中文 名称.txt"), "y\n").unwrap();
        let st = git_status(root.to_str().unwrap()).unwrap();
        let paths: Vec<&str> = st.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"my file #1.txt"), "actual: {paths:?}");
        assert!(paths.contains(&"中文 名称.txt"), "actual: {paths:?}");
    }

    #[test]
    fn status_shows_renamed_and_unmerged() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("old.txt"), "x\n").unwrap();
        init_committed_repo(root);
        // 纯重命名（git mv）→ Renamed，展示新路径
        assert_eq!(git(root, &["mv", "old.txt", "new.txt"]).0, 0);
        let st = git_status(root.to_str().unwrap()).unwrap();
        let r = st.files.iter().find(|f| f.path == "new.txt").unwrap();
        assert_eq!(r.status, FileStatus::Renamed);
        assert!(r.staged);
        assert!(!r.worktree);

        // 制造未合并索引 → Conflicted（staged + worktree 双标志）
        assert_eq!(git(root, &["commit", "-m", "rename"]).0, 0);
        assert_eq!(git(root, &["checkout", "-b", "side"]).0, 0);
        std::fs::write(root.join("new.txt"), "side\n").unwrap();
        assert_eq!(git(root, &["add", "new.txt"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "side"]).0, 0);
        assert_eq!(git(root, &["checkout", "main"]).0, 0);
        std::fs::write(root.join("new.txt"), "main\n").unwrap();
        assert_eq!(git(root, &["add", "new.txt"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "main"]).0, 0);
        let (code, _, _) = git(root, &["merge", "side"]);
        assert_ne!(code, 0, "merge 应产生冲突");
        let st = git_status(root.to_str().unwrap()).unwrap();
        let c = st.files.iter().find(|f| f.path == "new.txt").unwrap();
        assert_eq!(c.status, FileStatus::Conflicted);
        assert!(c.staged);
        assert!(c.worktree);
        // 清理冲突状态
        assert_eq!(git(root, &["merge", "--abort"]).0, 0);
    }

    #[test]
    fn init_creates_repo_without_commit() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let st = git_init(root.to_str().unwrap()).unwrap();
        assert!(root.join(".git").is_dir());
        // 默认创建 .gitignore / .gitattributes：按小写路径排序在前，均为未跟踪
        assert_eq!(
            st.files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec![".gitattributes", ".gitignore", "a.txt"]
        );
        assert!(st
            .files
            .iter()
            .all(|f| f.status == FileStatus::Untracked && f.worktree && !f.staged));
        let ig = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(ig.contains("node_modules/"));
        assert!(ig.contains(".idea/"));
        assert!(ig.contains(".vs/"));
        let attrs = std::fs::read_to_string(root.join(".gitattributes")).unwrap();
        assert_eq!(attrs, "* text=auto\n");
        // 仅初始化，不应产生任何提交（HEAD 未出生）
        assert_ne!(
            git(root, &["rev-parse", "--verify", "--quiet", "HEAD"]).0,
            0,
            "HEAD 应未出生"
        );
    }

    #[test]
    fn init_preserves_existing_gitignore_and_gitattributes() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "custom-ignore\n").unwrap();
        std::fs::write(root.join(".gitattributes"), "*.bin binary\n").unwrap();
        let st = git_init(root.to_str().unwrap()).unwrap();
        assert!(root.join(".git").is_dir());
        // 已有文件内容原样保留，不被默认模板覆盖
        assert_eq!(
            std::fs::read_to_string(root.join(".gitignore")).unwrap(),
            "custom-ignore\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join(".gitattributes")).unwrap(),
            "*.bin binary\n"
        );
        // 两个已有文件仍以未跟踪状态出现（不重复创建）
        assert_eq!(st.files.len(), 2);
        assert!(st.files.iter().all(|f| f.status == FileStatus::Untracked));
    }

    #[test]
    fn init_twice_errors() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let _ = git_init(root.to_str().unwrap()).unwrap();
        let err = git_init(root.to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, GitErrorCode::RepoError);
        assert!(err.message.contains("已经是 Git 仓库"));
    }

    #[test]
    fn status_not_a_repo() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("x.txt"), "x").unwrap();
        let err = git_status(dir.path().to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, GitErrorCode::NotARepo);
    }

    #[test]
    fn diff_untracked_returns_new_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("c.txt"), "hello\n").unwrap();
        let st = git_init(root.to_str().unwrap()).unwrap();
        let diff = git_diff(&st.repo_workspace, "c.txt", "untracked").unwrap();
        assert!(diff.contains("@@"));
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
        let st = git_status(root.to_str().unwrap()).unwrap();
        let diff = git_diff(&st.repo_workspace, "a.txt", "modified").unwrap();
        assert!(diff.contains("@@"));
        assert!(diff.contains("-two"));
        assert!(diff.contains("+two!"));
    }

    #[test]
    fn diff_deleted_returns_removals() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        init_committed_repo(root);
        std::fs::remove_file(root.join("a.txt")).unwrap();
        let st = git_status(root.to_str().unwrap()).unwrap();
        let diff = git_diff(&st.repo_workspace, "a.txt", "deleted").unwrap();
        assert!(diff.contains("-one"));
        assert!(diff.contains("-two"));
        assert!(!diff.contains("+one"));
    }

    #[test]
    fn diff_binary_errors() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("bin.dat"), b"\x00\x01\x02").unwrap();
        let st = git_init(root.to_str().unwrap()).unwrap();
        let err = git_diff(&st.repo_workspace, "bin.dat", "untracked").unwrap_err();
        assert!(err.message.contains("二进制"));
    }

    /// 构造“前 8000 字节为纯文本、之后夹带 NUL”的内容：
    /// 资源面板探测为文本可编辑，而 git 自身（全文件扫描 NUL）判为二进制。
    fn late_nul_content() -> Vec<u8> {
        let mut content = b"line text\n".repeat(1200); // 前 12000 字节为纯文本
        content.extend_from_slice(&[0u8, 1, 2, 3].repeat(100)); // 之后夹带 NUL
        content.push(b'\n'); // NUL 行后换行，保证后续断言行独立成行
        content.extend_from_slice(b"AFTER-BEFORE\ntail line\n");
        content
    }

    #[test]
    fn diff_modified_with_late_nul_returns_text() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let mut content = late_nul_content();
        std::fs::write(root.join("mix.dat"), &content).unwrap();
        init_committed_repo(root);
        let needle = b"AFTER-BEFORE";
        let pos = content.windows(needle.len()).position(|w| w == needle).unwrap();
        content.splice(pos..pos + needle.len(), b"AFTER-CHANGED".iter().copied());
        std::fs::write(root.join("mix.dat"), &content).unwrap();
        let st = git_status(root.to_str().unwrap()).unwrap();
        let diff = git_diff(&st.repo_workspace, "mix.dat", "modified").unwrap();
        assert!(diff.contains("+AFTER-CHANGED"));
        assert!(diff.contains("-AFTER-BEFORE"));
    }

    #[test]
    fn diff_gitattributes_binary_returns_text() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("doc.txt"), "plain line 1\nplain line 2\n").unwrap();
        std::fs::write(root.join(".gitattributes"), "doc.txt binary\n").unwrap();
        init_committed_repo(root);
        std::fs::write(root.join("doc.txt"), "plain line 1\nplain line 2\nchanged\n").unwrap();
        let st = git_status(root.to_str().unwrap()).unwrap();
        let diff = git_diff(&st.repo_workspace, "doc.txt", "modified").unwrap();
        assert!(diff.contains("+changed"));
    }

    #[test]
    fn diff_untracked_with_late_nul_returns_text() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("mix.dat"), late_nul_content()).unwrap();
        let st = git_init(root.to_str().unwrap()).unwrap();
        let diff = git_diff(&st.repo_workspace, "mix.dat", "untracked").unwrap();
        assert!(diff.contains("+AFTER-BEFORE"));
    }

    #[test]
    fn status_does_not_write_index() {
        // 只读状态：初始化后（无 index 文件）执行状态不应创建 .git/index
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();
        assert!(!root.join(".git").join("index").exists());
        let st = git_status(root.to_str().unwrap()).unwrap();
        assert!(!root.join(".git").join("index").exists());
        // git_init 默认创建 .gitignore / .gitattributes，随 a.txt 共 3 个未跟踪文件
        assert_eq!(st.files.len(), 3);
    }

    #[test]
    fn path_excludes_dot_dirs_but_keeps_git_and_dot_files() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".vs")).unwrap();
        std::fs::create_dir_all(root.join(".vscode")).unwrap();
        std::fs::create_dir_all(root.join("src/.cache")).unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::create_dir_all(root.join("src/node_modules")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "x").unwrap();
        std::fs::write(root.join("src/a.txt"), "x").unwrap();
        std::fs::write(root.join(".git/index"), "x").unwrap();

        assert!(path_under_excluded_dot_dir(root, &root.join(".vs/foo")));
        assert!(path_under_excluded_dot_dir(root, &root.join("src/.cache/x")));
        assert!(path_under_excluded_dot_dir(root, &root.join(".vscode/settings.json")));
        assert!(path_under_excluded_dot_dir(root, &root.join("node_modules/pkg/index.js")));
        assert!(path_under_excluded_dot_dir(root, &root.join("src/node_modules/a.js")));
        assert!(!path_under_excluded_dot_dir(root, &root.join(".git/index")));
        assert!(!path_under_excluded_dot_dir(root, &root.join(".gitignore")));
        assert!(!path_under_excluded_dot_dir(root, &root.join("src/a.txt")));
    }

    #[test]
    fn parse_ignored_list_splits_dirs_and_files() {
        let snap = parse_ignored_list("dist/\0debug.log\0src/gen/\0");
        assert!(snap.dirs.contains("dist"));
        assert!(snap.dirs.contains("src/gen"));
        assert!(snap.files.contains("debug.log"));
        assert!(!snap.files.contains("dist"));
    }

    #[test]
    fn is_ignored_matches_files_and_ancestor_dirs() {
        let snap = parse_ignored_list("dist/\0debug.log\0");
        assert!(snap.is_ignored("debug.log"));
        assert!(snap.is_ignored("dist"));
        assert!(snap.is_ignored("dist/x/y.txt"));
        // 大小写/分隔符归一（Windows 大小写不敏感）
        assert!(snap.is_ignored("DIST\\x.txt"));
        assert!(!snap.is_ignored("src/main.rs"));
        assert!(!snap.is_ignored("distx.txt"));
    }

    #[test]
    fn build_ignore_snapshot_uses_gitignore() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "ignored.txt\ntmp/\n").unwrap();
        std::fs::write(root.join("ignored.txt"), "x").unwrap();
        std::fs::create_dir_all(root.join("tmp/sub")).unwrap();
        std::fs::write(root.join("tmp/sub/y.txt"), "y").unwrap();
        std::fs::write(root.join("keep.txt"), "k").unwrap();
        assert_eq!(git(root, &["init", "-b", "main"]).0, 0);

        let git_bin = PathBuf::from("git");
        let root_s = root.to_str().unwrap();
        let snap = build_ignore_snapshot(&git_bin, root_s);
        assert!(snap.files.contains("ignored.txt"));
        assert!(snap.dirs.contains("tmp"));
        assert!(!snap.files.contains("keep.txt"));

        // 删除 .gitignore 后重建为空（不再有忽略规则）
        std::fs::remove_file(root.join(".gitignore")).unwrap();
        let snap2 = build_ignore_snapshot(&git_bin, root_s);
        assert!(snap2.files.is_empty());
        assert!(snap2.dirs.is_empty());
    }

    #[test]
    fn validate_branch_name_rules() {
        assert!(validate_branch_name("dev").is_ok());
        assert!(validate_branch_name("feature/foo").is_ok());
        assert!(validate_branch_name("中文分支").is_ok());
        assert!(validate_branch_name("").is_err());
        assert!(validate_branch_name("-x").is_err());
        assert!(validate_branch_name("/x").is_err());
        assert!(validate_branch_name("x/").is_err());
        assert!(validate_branch_name("x.").is_err());
        assert!(validate_branch_name("a b").is_err());
        assert!(validate_branch_name("a..b").is_err());
        assert!(validate_branch_name("a~b").is_err());
        assert!(validate_branch_name("a@{b").is_err());
    }

    #[test]
    fn branches_lists_local_branches_and_current() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        assert_eq!(git(root, &["branch", "feature"]).0, 0);
        let bs = git_branch(root.to_str().unwrap()).unwrap();
        assert_eq!(bs.current, "main");
        assert_eq!(bs.branches, vec!["feature".to_string(), "main".to_string()]);
    }

    #[test]
    fn branch_create_from_head_keeps_current() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        let st = git_branch_create(root.to_str().unwrap(), "dev").unwrap();
        assert_eq!(st.branch, "main");
        let bs = git_branch(root.to_str().unwrap()).unwrap();
        assert!(bs.branches.contains(&"dev".to_string()));
        // 重复创建同名分支报错
        assert!(git_branch_create(root.to_str().unwrap(), "dev").is_err());
    }

    #[test]
    fn branch_create_requires_commit() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();
        let err = git_branch_create(root.to_str().unwrap(), "dev").unwrap_err();
        assert!(err.message.contains("还没有任何提交"));
    }

    #[test]
    fn branch_delete_removes_and_guards_current() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        assert_eq!(git(root, &["branch", "feature"]).0, 0);
        let st = git_branch_delete(root.to_str().unwrap(), "feature").unwrap();
        assert_eq!(st.branch, "main");
        let bs = git_branch(root.to_str().unwrap()).unwrap();
        assert!(!bs.branches.contains(&"feature".to_string()));
        // 禁止删除当前分支
        let err = git_branch_delete(root.to_str().unwrap(), "main").unwrap_err();
        assert!(err.message.contains("当前所在分支"));
        // 删除不存在分支报错
        assert!(git_branch_delete(root.to_str().unwrap(), "nope").is_err());
    }

    #[test]
    fn branch_delete_unmerged_succeeds() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        // feature 有独立提交，未合并回 main
        assert_eq!(git(root, &["checkout", "-b", "feature"]).0, 0);
        std::fs::write(root.join("b.txt"), "feat\n").unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "feat"]).0, 0);
        assert_eq!(git(root, &["checkout", "main"]).0, 0);

        // 无条件删除：未完全合并的分支也应删除成功
        let st = git_branch_delete(root.to_str().unwrap(), "feature").unwrap();
        assert_eq!(st.branch, "main");
        let bs = git_branch(root.to_str().unwrap()).unwrap();
        assert!(!bs.branches.contains(&"feature".to_string()));
        assert!(bs.branches.contains(&"main".to_string()));
    }

    #[test]
    fn branch_switch_updates_worktree_head_and_index() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one").unwrap();
        init_committed_repo(root);
        // 夹具：在 other 分支提交不同内容
        assert_eq!(git(root, &["branch", "other"]).0, 0);
        assert_eq!(git(root, &["switch", "other"]).0, 0);
        std::fs::write(root.join("a.txt"), "two").unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "other"]).0, 0);
        assert_eq!(git(root, &["switch", "main"]).0, 0);
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one");

        // 进程内切换到 other：工作区/索引/HEAD 同步
        let st = git_switch(root.to_str().unwrap(), "other").unwrap();
        assert_eq!(st.branch, "other");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "two");
        assert!(st.files.is_empty());

        // 切回 main
        let st = git_switch(root.to_str().unwrap(), "main").unwrap();
        assert_eq!(st.branch, "main");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one");
    }

    #[test]
    fn branch_switch_carries_uncommitted_changes() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        assert_eq!(git(root, &["branch", "other"]).0, 0);
        // 本地修改 a.txt（other 分支未触及）→ 切换成功并携带
        std::fs::write(root.join("a.txt"), "dirty").unwrap();
        let st = git_switch(root.to_str().unwrap(), "other").unwrap();
        assert_eq!(st.branch, "other");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "dirty");
        assert!(st.files.iter().any(|f| f.path == "a.txt"));
        // 切回 main：改动继续携带
        let st = git_switch(root.to_str().unwrap(), "main").unwrap();
        assert_eq!(st.branch, "main");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "dirty");
    }

    #[test]
    fn branch_switch_refuses_conflicting_local_changes() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        // other 分支把 a.txt 改为 two
        assert_eq!(git(root, &["branch", "other"]).0, 0);
        assert_eq!(git(root, &["switch", "other"]).0, 0);
        std::fs::write(root.join("a.txt"), "two").unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "other"]).0, 0);
        assert_eq!(git(root, &["switch", "main"]).0, 0);
        // main 本地修改 a.txt 与目标不同 → 拒绝且停留在 main
        std::fs::write(root.join("a.txt"), "local").unwrap();
        let err = git_switch(root.to_str().unwrap(), "other").unwrap_err();
        assert!(err.message.contains("本地修改与目标分支冲突"));
        assert!(err.message.contains("a.txt"));
        assert_eq!(git(root, &["branch", "--show-current"]).1.trim(), "main");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "local");
    }

    #[test]
    fn branch_switch_refuses_when_local_changes_would_be_overwritten() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        assert_eq!(git(root, &["branch", "other"]).0, 0);
        assert_eq!(git(root, &["switch", "other"]).0, 0);
        std::fs::write(root.join("a.txt"), "two").unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "other"]).0, 0);
        assert_eq!(git(root, &["switch", "main"]).0, 0);
        // 本地内容与 HEAD 不一致（即使与目标一致）→ 标准 git 仍拒绝，
        // 避免任何未提交改动被覆盖的歧义；停留在 main
        std::fs::write(root.join("a.txt"), "two").unwrap();
        let err = git_switch(root.to_str().unwrap(), "other").unwrap_err();
        assert!(err.message.contains("本地修改与目标分支冲突"));
        assert_eq!(git(root, &["branch", "--show-current"]).1.trim(), "main");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "two");
    }

    #[test]
    fn branch_switch_carries_local_deletion() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        assert_eq!(git(root, &["branch", "other"]).0, 0);
        // 本地删除 a.txt（other 未触及）→ 切换成功且删除携带
        std::fs::remove_file(root.join("a.txt")).unwrap();
        let st = git_switch(root.to_str().unwrap(), "other").unwrap();
        assert_eq!(st.branch, "other");
        assert!(!root.join("a.txt").exists());
        assert!(st.files.iter().any(|f| f.path == "a.txt"));
    }

    #[test]
    fn branch_switch_refuses_target_deletes_locally_modified() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        // other 分支删除 a.txt
        assert_eq!(git(root, &["branch", "other"]).0, 0);
        assert_eq!(git(root, &["switch", "other"]).0, 0);
        assert_eq!(git(root, &["rm", "a.txt"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "del"]).0, 0);
        assert_eq!(git(root, &["switch", "main"]).0, 0);
        // main 本地修改 a.txt，目标删除 → 拒绝
        std::fs::write(root.join("a.txt"), "local").unwrap();
        let err = git_switch(root.to_str().unwrap(), "other").unwrap_err();
        assert!(err.message.contains("本地修改与目标分支冲突"));
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "local");
    }

    #[test]
    fn branch_switch_allows_when_both_deleted() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        assert_eq!(git(root, &["branch", "other"]).0, 0);
        assert_eq!(git(root, &["switch", "other"]).0, 0);
        assert_eq!(git(root, &["rm", "a.txt"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "del"]).0, 0);
        assert_eq!(git(root, &["switch", "main"]).0, 0);
        // 本地也删除 a.txt → 双方一致，切换成功
        std::fs::remove_file(root.join("a.txt")).unwrap();
        let st = git_switch(root.to_str().unwrap(), "other").unwrap();
        assert_eq!(st.branch, "other");
        assert!(!root.join("a.txt").exists());
    }

    #[test]
    fn branch_switch_preserves_staged_changes_on_untouched_paths() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        assert_eq!(git(root, &["branch", "other"]).0, 0);
        // 修改 a.txt 并暂存；other 分支未触及 → 切换后索引保留暂存内容
        std::fs::write(root.join("a.txt"), "staged").unwrap();
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);
        let st = git_switch(root.to_str().unwrap(), "other").unwrap();
        assert_eq!(st.branch, "other");
        let (_, blob, _) = git(root, &["show", ":a.txt"]);
        assert_eq!(blob, "staged");
    }

    #[test]
    fn branch_switch_refuses_overwriting_untracked() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        // other 分支提交 x.txt
        assert_eq!(git(root, &["branch", "other"]).0, 0);
        assert_eq!(git(root, &["switch", "other"]).0, 0);
        std::fs::write(root.join("x.txt"), "committed").unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "x"]).0, 0);
        assert_eq!(git(root, &["switch", "main"]).0, 0);
        // main 工作区出现未跟踪 x.txt，切换将覆盖 → 拒绝
        std::fs::write(root.join("x.txt"), "local").unwrap();
        let err = git_switch(root.to_str().unwrap(), "other").unwrap_err();
        assert!(err.message.contains("未跟踪文件将被覆盖"));
    }

    #[test]
    fn branch_switch_allows_differing_directory_between_branches() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/a.txt"), "one").unwrap();
        init_committed_repo(root);
        // other 分支修改 docs/a.txt，形成目录级树差异
        assert_eq!(git(root, &["branch", "other"]).0, 0);
        assert_eq!(git(root, &["switch", "other"]).0, 0);
        std::fs::write(root.join("docs/a.txt"), "two").unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "other"]).0, 0);
        assert_eq!(git(root, &["switch", "main"]).0, 0);

        // 进程内切换到 other：docs 目录不应被误判为未跟踪文件
        let st = git_switch(root.to_str().unwrap(), "other").unwrap();
        assert_eq!(st.branch, "other");
        assert_eq!(std::fs::read_to_string(root.join("docs/a.txt")).unwrap(), "two");

        // 切回 main
        let st = git_switch(root.to_str().unwrap(), "main").unwrap();
        assert_eq!(st.branch, "main");
        assert_eq!(std::fs::read_to_string(root.join("docs/a.txt")).unwrap(), "one");
    }

    #[test]
    fn branch_switch_refuses_untracked_file_inside_new_directory() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        // other 分支新增 newdir/x.txt
        assert_eq!(git(root, &["branch", "other"]).0, 0);
        assert_eq!(git(root, &["switch", "other"]).0, 0);
        std::fs::create_dir_all(root.join("newdir")).unwrap();
        std::fs::write(root.join("newdir/x.txt"), "committed").unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "newdir"]).0, 0);
        assert_eq!(git(root, &["switch", "main"]).0, 0);
        // main 工作区出现未跟踪 newdir/x.txt，切换将覆盖 → 拒绝
        std::fs::create_dir_all(root.join("newdir")).unwrap();
        std::fs::write(root.join("newdir/x.txt"), "local").unwrap();
        let err = git_switch(root.to_str().unwrap(), "other").unwrap_err();
        assert!(err.message.contains("未跟踪文件将被覆盖"));
        assert!(err.message.contains("newdir/x.txt"));
    }

    #[test]
    fn stage_then_unstage_untracked_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("c.txt"), "hello\n").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();

        // 暂存 → added 且 staged=true
        let st = git_add(root.to_str().unwrap(), "c.txt").unwrap();
        assert_eq!(st.files.len(), 3);
        let c = st.files.iter().find(|f| f.path == "c.txt").unwrap();
        assert_eq!(c.status, FileStatus::Added);
        assert!(c.staged);
        assert!(!c.worktree);

        // 取消暂存 → 回到 untracked 且 staged=false，工作区内容保留
        let st = git_unstage(root.to_str().unwrap(), "c.txt").unwrap();
        assert_eq!(st.files.len(), 3);
        let c = st.files.iter().find(|f| f.path == "c.txt").unwrap();
        assert_eq!(c.status, FileStatus::Untracked);
        assert!(!c.staged);
        assert!(c.worktree);
        assert_eq!(std::fs::read_to_string(root.join("c.txt")).unwrap(), "hello\n");
    }

    #[test]
    fn stage_modified_then_unstage_keeps_worktree() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        std::fs::write(root.join("a.txt"), "two\n").unwrap();

        let st = git_add(root.to_str().unwrap(), "a.txt").unwrap();
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].status, FileStatus::Modified);
        assert!(st.files[0].staged);
        assert!(!st.files[0].worktree);

        let st = git_unstage(root.to_str().unwrap(), "a.txt").unwrap();
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].status, FileStatus::Modified);
        assert!(!st.files[0].staged);
        assert!(st.files[0].worktree);
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "two\n");
    }

    #[test]
    fn stage_deleted_file_records_deletion() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        std::fs::remove_file(root.join("a.txt")).unwrap();

        let st = git_add(root.to_str().unwrap(), "a.txt").unwrap();
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].path, "a.txt");
        assert_eq!(st.files[0].status, FileStatus::Deleted);
        assert!(st.files[0].staged);
        assert!(!st.files[0].worktree);
    }

    #[test]
    fn stage_all_moves_all_worktree_changes_to_staged() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        std::fs::write(root.join("c.txt"), "gone\n").unwrap();
        init_committed_repo(root);
        // 工作区侧：修改 + 未跟踪 + 删除
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        std::fs::write(root.join("new.txt"), "hello\n").unwrap();
        std::fs::remove_file(root.join("c.txt")).unwrap();

        let st = git_add_all(root.to_str().unwrap()).unwrap();
        assert_eq!(st.files.len(), 3);
        for f in &st.files {
            assert!(f.staged, "{} 应已暂存", f.path);
            assert!(!f.worktree, "{} 不应再有工作区侧变更", f.path);
        }
        let by_path = |p: &str| {
            st.files
                .iter()
                .find(|f| f.path == p)
                .map(|f| f.status)
                .unwrap()
        };
        assert_eq!(by_path("a.txt"), FileStatus::Modified);
        assert_eq!(by_path("new.txt"), FileStatus::Added);
        assert_eq!(by_path("c.txt"), FileStatus::Deleted);
    }

    #[test]
    fn unstage_all_returns_files_to_worktree() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        std::fs::write(root.join("new.txt"), "hello\n").unwrap();

        let st = git_add_all(root.to_str().unwrap()).unwrap();
        assert!(st.files.iter().all(|f| f.staged));

        let st = git_unstage_all(root.to_str().unwrap()).unwrap();
        assert_eq!(st.files.len(), 2);
        for f in &st.files {
            assert!(!f.staged, "{} 应已取消暂存", f.path);
            assert!(f.worktree, "{} 应回到工作区侧", f.path);
        }
        // 工作区内容原样保留
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "two\n");
        assert_eq!(std::fs::read_to_string(root.join("new.txt")).unwrap(), "hello\n");
    }

    #[test]
    fn stage_all_and_unstage_all_are_noop_when_clean() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);

        assert!(git_add_all(root.to_str().unwrap()).unwrap().files.is_empty());
        assert!(git_unstage_all(root.to_str().unwrap())
            .unwrap()
            .files
            .is_empty());
    }

    #[test]
    fn restore_discards_worktree_and_staged_changes() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        // 暂存新内容后再修改工作区：同一路径 staged + worktree 都有变化
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);
        std::fs::write(root.join("a.txt"), "three\n").unwrap();

        let st = git_restore(root.to_str().unwrap(), "a.txt").unwrap();
        assert!(st.files.is_empty());
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\n");
    }

    #[test]
    fn restore_untracked_file_deletes_it() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("c.txt"), "hello\n").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();

        let st = git_restore(root.to_str().unwrap(), "c.txt").unwrap();
        // c.txt 已删除；默认创建的 .gitignore / .gitattributes 仍为未跟踪
        assert_eq!(st.files.len(), 2);
        assert!(st.files.iter().all(|f| f.status == FileStatus::Untracked));
        assert!(!root.join("c.txt").exists());
    }

    #[test]
    fn delete_removes_file_and_index_entry() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        // 先暂存修改，再删除：等价 git rm（工作区 + 索引同时移除）
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);

        let st = git_rm(root.to_str().unwrap(), "a.txt").unwrap();
        // 等价 git rm：删除工作区文件并在暂存区记录删除
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].path, "a.txt");
        assert_eq!(st.files[0].status, FileStatus::Deleted);
        assert!(st.files[0].staged);
        assert!(!st.files[0].worktree);
        assert!(!root.join("a.txt").exists());
    }

    #[test]
    fn ignore_adds_gitignore_entry_idempotently() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("c.txt"), "hello\n").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();

        let st = git_ignore(root.to_str().unwrap(), "c.txt").unwrap();
        // c.txt 已被忽略；默认创建的 .gitignore / .gitattributes 仍为未跟踪
        assert_eq!(st.files.len(), 2);
        let ig_file = st.files.iter().find(|f| f.path == ".gitignore").unwrap();
        assert!(!ig_file.staged);
        assert!(ig_file.worktree);
        let ig = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(ig.contains("node_modules/"));
        assert!(ig.ends_with("/c.txt\n"));

        // 重复忽略幂等：条目不重复
        let _ = git_ignore(root.to_str().unwrap(), "c.txt").unwrap();
        let ig = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(ig.matches("/c.txt\n").count(), 1);
    }

    #[test]
    fn ignore_appends_to_existing_gitignore_without_trailing_newline() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("c.txt"), "hello\n").unwrap();
        std::fs::write(root.join("d.txt"), "world\n").unwrap();
        std::fs::write(root.join(".gitignore"), "*.log").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();

        let st = git_ignore(root.to_str().unwrap(), "c.txt").unwrap();
        // c.txt 被忽略；d.txt 与 .gitignore 仍为未跟踪
        let mut paths: Vec<&str> = st.files.iter().map(|f| f.path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, vec![".gitattributes", ".gitignore", "d.txt"]);
        let ig = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(ig, "*.log\n/c.txt\n");
    }

    #[test]
    fn file_ops_reject_unsafe_paths() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();

        assert!(git_add(root.to_str().unwrap(), "../outside").is_err());
        assert!(git_add(root.to_str().unwrap(), r"C:\outside").is_err());
        assert!(git_add(root.to_str().unwrap(), "node_modules/x.js").is_err());
        assert!(git_add(root.to_str().unwrap(), ".git/config").is_err());
        assert!(git_restore(root.to_str().unwrap(), "a/../b").is_err());
        assert!(git_rm(root.to_str().unwrap(), "/abs").is_err());
        assert!(git_ignore(root.to_str().unwrap(), "").is_err());
    }

    #[test]
    fn git_diff_rejects_unsafe_paths() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();

        assert!(git_diff(root.to_str().unwrap(), "../outside.txt", "modified").is_err());
        assert!(git_diff(root.to_str().unwrap(), r"C:\outside.txt", "modified").is_err());
        assert!(git_diff(root.to_str().unwrap(), "node_modules/x.js", "modified").is_err());
        assert!(git_diff(root.to_str().unwrap(), "a/../b", "modified").is_err());
    }

    #[test]
    fn commit_refuses_unmerged_index() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "base\n").unwrap();
        init_committed_repo(root);
        // side 分支修改同一文件后合并，制造 unmerged 索引
        assert_eq!(git(root, &["checkout", "-b", "side"]).0, 0);
        std::fs::write(root.join("a.txt"), "side\n").unwrap();
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "side"]).0, 0);
        assert_eq!(git(root, &["checkout", "main"]).0, 0);
        std::fs::write(root.join("a.txt"), "main\n").unwrap();
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "main"]).0, 0);
        let (code, _, _) = git(root, &["merge", "side"]);
        assert_ne!(code, 0, "merge 应产生冲突");

        let err = git_commit(root.to_str().unwrap(), "msg").unwrap_err();
        assert!(err.message.contains("冲突"));
    }

    #[test]
    fn stage_rejects_oversized_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();

        std::fs::write(root.join("big.bin"), vec![0u8; MAX_FILE_BYTES as usize + 1]).unwrap();
        let err = git_add(root.to_str().unwrap(), "big.bin").unwrap_err();
        assert!(err.message.contains("文件过大"));
    }

    #[test]
    fn stage_untracked_directory_recursively() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        std::fs::create_dir_all(root.join("newdir/sub")).unwrap();
        std::fs::write(root.join("newdir/a.txt"), "x\n").unwrap();
        std::fs::write(root.join("newdir/sub/b.txt"), "y\n").unwrap();
        std::fs::write(root.join("newdir/x.log"), "ignored\n").unwrap();

        let st = git_add(root.to_str().unwrap(), "newdir").unwrap();
        let mut by_path: Vec<(String, FileStatus, bool, bool)> = st
            .files
            .iter()
            .map(|f| (f.path.clone(), f.status, f.staged, f.worktree))
            .collect();
        by_path.sort();
        assert_eq!(
            by_path,
            vec![
                (
                    ".gitignore".to_string(),
                    FileStatus::Untracked,
                    false,
                    true
                ),
                (
                    "newdir/a.txt".to_string(),
                    FileStatus::Added,
                    true,
                    false
                ),
                (
                    "newdir/sub/b.txt".to_string(),
                    FileStatus::Added,
                    true,
                    false
                ),
            ]
        );
        // 已忽略文件不应被加入索引
        let (_, ls, _) = git(root, &["ls-files"]);
        assert!(!ls.contains("x.log"), "已忽略文件不应被加入索引: {ls}");
    }

    #[test]
    fn stage_directory_matches_only_exact_prefix() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("src2")).unwrap();
        std::fs::write(root.join("src/a.txt"), "x\n").unwrap();
        std::fs::write(root.join("src2/b.txt"), "y\n").unwrap();

        let st = git_add(root.to_str().unwrap(), "src").unwrap();
        let mut by_path: Vec<(String, FileStatus, bool, bool)> = st
            .files
            .iter()
            .map(|f| (f.path.clone(), f.status, f.staged, f.worktree))
            .collect();
        by_path.sort();
        assert_eq!(
            by_path,
            vec![
                ("src/a.txt".to_string(), FileStatus::Added, true, false),
                (
                    "src2/b.txt".to_string(),
                    FileStatus::Untracked,
                    false,
                    true
                ),
            ]
        );
    }

    #[test]
    fn unstage_directory_resets_all() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.txt"), "one\n").unwrap();
        std::fs::write(root.join("src/b.txt"), "one\n").unwrap();
        init_committed_repo(root);
        std::fs::write(root.join("src/a.txt"), "two\n").unwrap();
        std::fs::write(root.join("src/b.txt"), "two\n").unwrap();

        let st = git_add(root.to_str().unwrap(), "src").unwrap();
        assert_eq!(st.files.len(), 2);
        assert!(st.files.iter().all(|f| f.staged));
        assert!(st.files.iter().all(|f| !f.worktree));

        let st = git_unstage(root.to_str().unwrap(), "src").unwrap();
        assert_eq!(st.files.len(), 2);
        assert!(st.files.iter().all(|f| !f.staged));
        assert!(st.files.iter().all(|f| f.worktree));
        assert_eq!(std::fs::read_to_string(root.join("src/a.txt")).unwrap(), "two\n");
        assert_eq!(std::fs::read_to_string(root.join("src/b.txt")).unwrap(), "two\n");
    }

    #[test]
    fn restore_directory_discards_all() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.txt"), "one\n").unwrap();
        std::fs::write(root.join("src/b.txt"), "one\n").unwrap();
        init_committed_repo(root);
        std::fs::write(root.join("src/a.txt"), "two\n").unwrap();
        std::fs::remove_file(root.join("src/b.txt")).unwrap();
        std::fs::write(root.join("src/c.txt"), "new\n").unwrap();

        let st = git_restore(root.to_str().unwrap(), "src").unwrap();
        assert!(st.files.is_empty());
        assert_eq!(std::fs::read_to_string(root.join("src/a.txt")).unwrap(), "one\n");
        assert_eq!(std::fs::read_to_string(root.join("src/b.txt")).unwrap(), "one\n");
        assert!(!root.join("src/c.txt").exists());
    }

    #[test]
    fn ignore_directory_appends_slash_pattern() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("newdir")).unwrap();
        std::fs::write(root.join("newdir/a.txt"), "x\n").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();

        let st = git_ignore(root.to_str().unwrap(), "newdir").unwrap();
        assert_eq!(st.files.len(), 2);
        let ig_file = st.files.iter().find(|f| f.path == ".gitignore").unwrap();
        assert!(!ig_file.staged);
        assert!(ig_file.worktree);
        let ig = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(ig.ends_with("/newdir/\n"));

        // 重复忽略幂等
        let _ = git_ignore(root.to_str().unwrap(), "newdir").unwrap();
        let ig = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(ig.matches("/newdir/\n").count(), 1);
    }

    #[test]
    fn delete_directory_rejected() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("newdir")).unwrap();
        std::fs::write(root.join("newdir/a.txt"), "x\n").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();

        let err = git_rm(root.to_str().unwrap(), "newdir").unwrap_err();
        assert!(err.message.contains("不支持删除目录"));
    }

    // ---------- 提交 ----------

    #[test]
    fn commit_creates_commit_and_clears_staged() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        // 未跟踪文件保持未提交
        std::fs::write(root.join("c.txt"), "new\n").unwrap();
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);

        let st = git_commit(root.to_str().unwrap(), "feat: update a").unwrap();
        assert!(st.files.iter().all(|f| !f.staged));
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].path, "c.txt");
        assert_eq!(st.files[0].status, FileStatus::Untracked);
        // CLI 验证提交内容与工作区一致
        assert_eq!(
            git(root, &["log", "-1", "--format=%s"]).1.trim(),
            "feat: update a"
        );
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "two\n");
        assert!(!git(root, &["ls-files", "c.txt"]).1.contains("c.txt"));
    }

    #[test]
    fn commit_initial_commit_without_parent() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello\n").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();
        assert_eq!(git(root, &["config", "user.name", "t"]).0, 0);
        assert_eq!(git(root, &["config", "user.email", "t@t"]).0, 0);
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);

        let st = git_commit(root.to_str().unwrap(), "first").unwrap();
        // 仅 a.txt 被提交；默认创建的 .gitignore / .gitattributes 保持未跟踪
        assert_eq!(st.files.len(), 2);
        assert!(st.files.iter().all(|f| f.status == FileStatus::Untracked));
        assert_eq!(git(root, &["log", "-1", "--format=%s"]).1.trim(), "first");
    }

    #[test]
    fn commit_empty_message_errors() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();
        let err = git_commit(root.to_str().unwrap(), "   ").unwrap_err();
        assert!(err.message.contains("提交消息不能为空"));
    }

    #[test]
    fn commit_without_staged_errors() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        let err = git_commit(root.to_str().unwrap(), "msg").unwrap_err();
        assert!(err.message.contains("没有已暂存的更改"));
    }

    #[test]
    fn commit_without_user_config_errors() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        // git 会按规则回退到全局/系统配置与身份环境变量；存在可解析身份时
        // 无法构造“缺失”场景，优雅跳过，避免屏蔽全局配置的正当回退行为。
        let dir_probe = TempDir::new().unwrap();
        let probe = dir_probe.path();
        let has_identity = !git(probe, &["config", "user.name"]).1.trim().is_empty()
            || !git(probe, &["config", "user.email"]).1.trim().is_empty()
            || std::env::var_os("GIT_AUTHOR_NAME").is_some()
            || std::env::var_os("GIT_AUTHOR_EMAIL").is_some()
            || std::env::var_os("GIT_COMMITTER_NAME").is_some()
            || std::env::var_os("GIT_COMMITTER_EMAIL").is_some();
        if has_identity {
            eprintln!("skip: 已配置 git 用户身份（含全局配置回退），跳过缺失场景");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x\n").unwrap();
        assert_eq!(git(root, &["init", "-b", "main"]).0, 0);
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);
        let err = git_commit(root.to_str().unwrap(), "msg").unwrap_err();
        assert!(err.message.contains("user.name"));
    }

    // ---------- 拉取 ----------

    /// 构造 pull 测试环境：bare 远端来自 work1（含 init 提交），work2 为远端克隆。
    fn setup_pull_repo(bare: &Path, work1: &Path, work2: &Path) {
        assert_eq!(git(work1, &["init", "-b", "main"]).0, 0);
        assert_eq!(git(work1, &["config", "user.name", "t"]).0, 0);
        assert_eq!(git(work1, &["config", "user.email", "t@t"]).0, 0);
        // 关闭 autocrlf，保证检出内容与提交字节一致（不受全局配置影响）
        assert_eq!(git(work1, &["config", "core.autocrlf", "false"]).0, 0);
        std::fs::write(work1.join("a.txt"), "v1\n").unwrap();
        std::fs::write(work1.join("b.txt"), "vb\n").unwrap();
        assert_eq!(git(work1, &["add", "a.txt", "b.txt"]).0, 0);
        assert_eq!(git(work1, &["commit", "-m", "init"]).0, 0);
        assert_eq!(
            git(work1, &["clone", "--bare", ".", bare.to_str().unwrap()]).0,
            0
        );
        // 在 clone 阶段就关闭 autocrlf，避免全局配置把检出内容写成 CRLF 导致仓库“脏”
        assert_eq!(
            git(work2, &["-c", "core.autocrlf=false", "clone", bare.to_str().unwrap(), "."]).0,
            0
        );
        assert_eq!(git(work2, &["config", "user.name", "t"]).0, 0);
        assert_eq!(git(work2, &["config", "user.email", "t@t"]).0, 0);
        assert_eq!(git(work2, &["config", "core.autocrlf", "false"]).0, 0);
    }

    #[test]
    fn pull_fast_forwards_when_behind() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        // work1 提交 v2 并推送到 bare → work2 落后
        std::fs::write(work1.path().join("a.txt"), "v2\n").unwrap();
        assert_eq!(git(work1.path(), &["add", "a.txt"]).0, 0);
        assert_eq!(git(work1.path(), &["commit", "-m", "v2"]).0, 0);
        assert_eq!(
            git(work1.path(), &["push", bare.path().to_str().unwrap(), "main"]).0,
            0
        );

        let res = git_pull(work2.path().to_str().unwrap()).unwrap();
        assert_eq!(res.kind, "fast_forward");
        assert!(res.status.files.is_empty());
        assert_eq!(
            std::fs::read_to_string(work2.path().join("a.txt")).unwrap(),
            "v2\n"
        );
        assert_eq!(git(work2.path(), &["log", "-1", "--format=%s"]).1.trim(), "v2");
    }

    #[test]
    fn pull_refuses_when_diverged() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        // work2 本地新增 b.txt 并提交（分叉）
        std::fs::write(work2.path().join("b.txt"), "local\n").unwrap();
        assert_eq!(git(work2.path(), &["add", "b.txt"]).0, 0);
        assert_eq!(git(work2.path(), &["commit", "-m", "local"]).0, 0);
        // work1 修改 a.txt 并推送到 bare
        std::fs::write(work1.path().join("a.txt"), "v2\n").unwrap();
        assert_eq!(git(work1.path(), &["add", "a.txt"]).0, 0);
        assert_eq!(git(work1.path(), &["commit", "-m", "v2"]).0, 0);
        assert_eq!(
            git(work1.path(), &["push", bare.path().to_str().unwrap(), "main"]).0,
            0
        );

        let before_head = git(work2.path(), &["rev-parse", "HEAD"]).1;
        let err = git_pull(work2.path().to_str().unwrap()).unwrap_err();
        assert!(err.message.contains("分叉"), "actual: {}", err.message);
        // ff-only 拒绝后不留任何冲突/合并残留
        assert_eq!(git(work2.path(), &["rev-parse", "HEAD"]).1, before_head);
        assert!(!work2.path().join(".git").join("MERGE_HEAD").exists());
        assert!(git(work2.path(), &["status", "--porcelain"]).1.is_empty());
    }

    #[test]
    fn pull_refuses_when_unstaged_overlaps() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        // work1 推送 a.txt 修改
        std::fs::write(work1.path().join("a.txt"), "v2\n").unwrap();
        assert_eq!(git(work1.path(), &["add", "a.txt"]).0, 0);
        assert_eq!(git(work1.path(), &["commit", "-m", "v2"]).0, 0);
        assert_eq!(
            git(work1.path(), &["push", bare.path().to_str().unwrap(), "main"]).0,
            0
        );
        // work2 未暂存修改同一文件 → 重叠拒绝
        std::fs::write(work2.path().join("a.txt"), "dirty\n").unwrap();
        let err = git_pull(work2.path().to_str().unwrap()).unwrap_err();
        assert!(err.message.contains("会被拉取覆盖"));
        // 拒绝后工作区与 HEAD 不变
        assert_eq!(
            std::fs::read_to_string(work2.path().join("a.txt")).unwrap(),
            "dirty\n"
        );
        assert_eq!(git(work2.path(), &["log", "-1", "--format=%s"]).1.trim(), "init");
    }

    #[test]
    fn pull_refuses_when_staged_overlaps() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        // work1 推送 a.txt 修改
        std::fs::write(work1.path().join("a.txt"), "v2\n").unwrap();
        assert_eq!(git(work1.path(), &["add", "a.txt"]).0, 0);
        assert_eq!(git(work1.path(), &["commit", "-m", "v2"]).0, 0);
        assert_eq!(
            git(work1.path(), &["push", bare.path().to_str().unwrap(), "main"]).0,
            0
        );
        // work2 暂存同一文件 → 重叠拒绝，索引/工作区/HEAD 均不变
        std::fs::write(work2.path().join("a.txt"), "staged\n").unwrap();
        assert_eq!(git(work2.path(), &["add", "a.txt"]).0, 0);
        let before_head = git(work2.path(), &["rev-parse", "HEAD"]).1;
        let err = git_pull(work2.path().to_str().unwrap()).unwrap_err();
        assert!(err.message.contains("会被拉取覆盖"));
        assert_eq!(
            std::fs::read_to_string(work2.path().join("a.txt")).unwrap(),
            "staged\n"
        );
        assert_eq!(git(work2.path(), &["rev-parse", "HEAD"]).1, before_head);
        let st = git_status(work2.path().to_str().unwrap()).unwrap();
        let a = st.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert!(a.staged);
    }

    #[test]
    fn pull_allows_staged_when_not_overlapping() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        // work1 推送 a.txt 修改；work2 暂存 b.txt 修改（不重叠）
        std::fs::write(work1.path().join("a.txt"), "v2\n").unwrap();
        assert_eq!(git(work1.path(), &["add", "a.txt"]).0, 0);
        assert_eq!(git(work1.path(), &["commit", "-m", "v2"]).0, 0);
        assert_eq!(
            git(work1.path(), &["push", bare.path().to_str().unwrap(), "main"]).0,
            0
        );
        std::fs::write(work2.path().join("b.txt"), "staged\n").unwrap();
        assert_eq!(git(work2.path(), &["add", "b.txt"]).0, 0);

        let res = git_pull(work2.path().to_str().unwrap()).unwrap();
        assert_eq!(res.kind, "fast_forward");
        assert_eq!(
            std::fs::read_to_string(work2.path().join("a.txt")).unwrap(),
            "v2\n"
        );
        // b.txt 的暂存修改仍在暂存区
        let b = res
            .status
            .files
            .iter()
            .find(|f| f.path == "b.txt")
            .unwrap();
        assert_eq!(b.status, FileStatus::Modified);
        assert!(b.staged);
        assert!(!b.worktree);
        let (code, _, _) = git(work2.path(), &["diff", "--cached", "--quiet", "b.txt"]);
        assert_ne!(code, 0);
    }

    #[test]
    fn pull_allows_unstaged_when_not_overlapping() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        // work1 推送 a.txt 修改；work2 未暂存修改 b.txt（不重叠）
        std::fs::write(work1.path().join("a.txt"), "v2\n").unwrap();
        assert_eq!(git(work1.path(), &["add", "a.txt"]).0, 0);
        assert_eq!(git(work1.path(), &["commit", "-m", "v2"]).0, 0);
        assert_eq!(
            git(work1.path(), &["push", bare.path().to_str().unwrap(), "main"]).0,
            0
        );
        std::fs::write(work2.path().join("b.txt"), "local\n").unwrap();
        // 未跟踪文件也应放行并保留
        std::fs::write(work2.path().join("c.txt"), "untracked\n").unwrap();

        let res = git_pull(work2.path().to_str().unwrap()).unwrap();
        assert_eq!(res.kind, "fast_forward");
        assert_eq!(
            std::fs::read_to_string(work2.path().join("a.txt")).unwrap(),
            "v2\n"
        );
        assert_eq!(
            std::fs::read_to_string(work2.path().join("b.txt")).unwrap(),
            "local\n"
        );
        assert_eq!(
            std::fs::read_to_string(work2.path().join("c.txt")).unwrap(),
            "untracked\n"
        );
        // b.txt 仍为已跟踪工作区修改；c.txt 仍为未跟踪
        let b = res
            .status
            .files
            .iter()
            .find(|f| f.path == "b.txt")
            .unwrap();
        assert_eq!(b.status, FileStatus::Modified);
        assert!(!b.staged);
        assert!(b.worktree);
        assert!(res
            .status
            .files
            .iter()
            .any(|f| f.path == "c.txt" && f.status == FileStatus::Untracked));
        let (code, _, _) = git(work2.path(), &["diff", "--quiet", "b.txt"]);
        assert_ne!(code, 0);
    }

    #[test]
    fn pull_up_to_date() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        let res = git_pull(work2.path().to_str().unwrap()).unwrap();
        assert_eq!(res.kind, "up_to_date");
        assert!(res.status.files.is_empty());
    }

    #[test]
    fn pull_reports_missing_git() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());

        let err = git_pull_with(
            Path::new("definitely-missing-git-binary-xyz"),
            work2.path().to_str().unwrap(),
        )
        .unwrap_err();
        assert!(
            err.message.contains("未检测到系统 git"),
            "actual: {}",
            err.message
        );
    }

    #[test]
    fn pull_fails_without_remote() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());

        let err = git_pull(work.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.message.contains("未找到可用的远端"),
            "actual: {}",
            err.message
        );
    }

    // ---------- 推送 ----------

    /// 推送成功：bare 远端 ref 更新、-u 设置上游（remote/merge）
    #[test]
    fn push_updates_remote_and_sets_upstream() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        // work2 本地提交 v2
        std::fs::write(work2.path().join("a.txt"), "v2\n").unwrap();
        assert_eq!(git(work2.path(), &["add", "a.txt"]).0, 0);
        assert_eq!(git(work2.path(), &["commit", "-m", "v2"]).0, 0);

        let res = git_push(work2.path().to_str().unwrap()).unwrap();
        assert_eq!(res.kind, "pushed");
        assert!(res.message.contains("已推送到"));
        // bare 远端 main 已指向 work2 HEAD
        let (code, remote_oid, _) = git(bare.path(), &["rev-parse", "main"]);
        assert_eq!(code, 0);
        let (_, local_oid, _) = git(work2.path(), &["rev-parse", "HEAD"]);
        assert_eq!(remote_oid.trim(), local_oid.trim());
        // 上游已设置
        let (_, remote, _) = git(work2.path(), &["config", "branch.main.remote"]);
        assert_eq!(remote.trim(), "origin");
        let (_, merge, _) = git(work2.path(), &["config", "branch.main.merge"]);
        assert_eq!(merge.trim(), "refs/heads/main");
    }

    #[test]
    fn push_up_to_date_second_time() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        std::fs::write(work2.path().join("a.txt"), "v2\n").unwrap();
        assert_eq!(git(work2.path(), &["add", "a.txt"]).0, 0);
        assert_eq!(git(work2.path(), &["commit", "-m", "v2"]).0, 0);
        assert_eq!(git_push(work2.path().to_str().unwrap()).unwrap().kind, "pushed");

        let res = git_push(work2.path().to_str().unwrap()).unwrap();
        assert_eq!(res.kind, "up_to_date");
        assert!(res.message.contains("已是最新"));
    }

    #[test]
    fn push_rejected_when_remote_ahead() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        // work1 推送到 bare，使远端领先
        std::fs::write(work1.path().join("a.txt"), "v2\n").unwrap();
        assert_eq!(git(work1.path(), &["add", "a.txt"]).0, 0);
        assert_eq!(git(work1.path(), &["commit", "-m", "v2"]).0, 0);
        assert_eq!(
            git(work1.path(), &["push", bare.path().to_str().unwrap(), "main"]).0,
            0
        );
        // work2 本地分叉提交
        std::fs::write(work2.path().join("b.txt"), "local\n").unwrap();
        assert_eq!(git(work2.path(), &["add", "b.txt"]).0, 0);
        assert_eq!(git(work2.path(), &["commit", "-m", "local"]).0, 0);

        let err = git_push(work2.path().to_str().unwrap()).unwrap_err();
        assert!(err.message.contains("先拉取"), "actual: {}", err.message);
    }

    #[test]
    fn push_rejects_detached_head() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        assert_eq!(git(work2.path(), &["checkout", "--detach"]).0, 0);

        let err = git_push(work2.path().to_str().unwrap()).unwrap_err();
        assert!(err.message.contains("游离"), "actual: {}", err.message);
    }

    #[test]
    fn push_fails_without_remote() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());

        let err = git_push(work.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.message.contains("未找到可用的远端"),
            "actual: {}",
            err.message
        );
    }

    #[test]
    fn push_reports_missing_git() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());

        let err = git_push_with(
            Path::new("definitely-missing-git-binary-xyz"),
            work2.path().to_str().unwrap(),
        )
        .unwrap_err();
        assert!(
            err.message.contains("未检测到系统 git"),
            "actual: {}",
            err.message
        );
    }

    #[test]
    fn git_available_detects_system_git() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        assert!(locate_git().is_some());
    }

    // ---------- 远端管理 ----------

    /// 归一化 Windows 路径后比较（git 存储远端 URL 时会把反斜杠转成斜杠）
    fn norm_url(s: &str) -> String {
        s.replace('\\', "/").trim_end_matches('/').to_lowercase()
    }

    #[test]
    fn remotes_lists_configured_remote_with_urls_and_current() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());

        let res = git_remote(work2.path().to_str().unwrap()).unwrap();
        assert_eq!(res.remotes.len(), 1);
        assert_eq!(res.remotes[0].name, "origin");
        let bare_url = bare.path().to_string_lossy();
        assert_eq!(
            norm_url(res.remotes[0].fetch_url.as_deref().unwrap_or_default()),
            norm_url(&bare_url)
        );
        assert_eq!(
            norm_url(res.remotes[0].push_url.as_deref().unwrap_or_default()),
            norm_url(&bare_url)
        );
        assert_eq!(res.current.as_deref(), Some("origin"));
    }

    #[test]
    fn remotes_empty_when_no_remote() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());

        let res = git_remote(work.path().to_str().unwrap()).unwrap();
        assert!(res.remotes.is_empty());
        assert_eq!(res.current, None);
    }

    #[test]
    fn remote_add_creates_remote_with_fetch_refspec() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());
        let bare = TempDir::new().unwrap();
        let git_bin = locate_git().unwrap();
        let url = bare.path().to_string_lossy();

        let res = git_remote_add_with(
            &git_bin,
            work.path().to_str().unwrap(),
            "upstream",
            &url,
        )
        .unwrap();
        assert_eq!(res.remotes.len(), 1);
        assert_eq!(res.remotes[0].name, "upstream");
        assert_eq!(
            norm_url(res.remotes[0].fetch_url.as_deref().unwrap_or_default()),
            norm_url(&url)
        );
        // git remote add 自动生成默认 fetch refspec
        let (code, fetch, _) = git(work.path(), &["config", "--get", "remote.upstream.fetch"]);
        assert_eq!(code, 0);
        assert!(fetch.contains("refs/remotes/upstream"), "actual: {fetch}");
    }

    #[test]
    fn remote_add_rejects_duplicate() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        let git_bin = locate_git().unwrap();

        let err = git_remote_add_with(
            &git_bin,
            work2.path().to_str().unwrap(),
            "origin",
            &bare.path().to_string_lossy(),
        )
        .unwrap_err();
        assert!(err.message.contains("已存在"), "actual: {}", err.message);
    }

    #[test]
    fn remote_add_validates_name_and_url() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());
        let git_bin = locate_git().unwrap();
        let root = work.path().to_str().unwrap();

        let err = git_remote_add_with(&git_bin, root, "bad name", "https://x").unwrap_err();
        assert!(err.message.contains("远端名"), "actual: {}", err.message);
        let err = git_remote_add_with(&git_bin, root, "up", "  ").unwrap_err();
        assert!(err.message.contains("远端地址"), "actual: {}", err.message);
    }

    #[test]
    fn remote_set_url_updates_fetch_address() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());
        let bare = TempDir::new().unwrap();
        let git_bin = locate_git().unwrap();
        let root = work.path().to_str().unwrap();
        let url = bare.path().to_string_lossy();
        let url2 = format!("{}/mirror", url);
        assert_eq!(
            git_remote_add_with(&git_bin, root, "origin", &url)
                .unwrap()
                .remotes
                .len(),
            1
        );

        let res = git_remote_set_url_with(&git_bin, root, "origin", &url2).unwrap();
        assert_eq!(
            norm_url(res.remotes[0].fetch_url.as_deref().unwrap_or_default()),
            norm_url(&url2)
        );
        let (code, got, _) = git(work.path(), &["remote", "get-url", "origin"]);
        assert_eq!(code, 0);
        assert_eq!(norm_url(got.trim()), norm_url(&url2));
    }

    #[test]
    fn remote_set_url_rejects_missing_remote() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());
        let git_bin = locate_git().unwrap();

        let err = git_remote_set_url_with(
            &git_bin,
            work.path().to_str().unwrap(),
            "nope",
            "https://example.com/x.git",
        )
        .unwrap_err();
        assert!(err.message.contains("远端不存在"), "actual: {}", err.message);
    }

    #[test]
    fn remote_remove_deletes_remote_and_section() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());
        let bare = TempDir::new().unwrap();
        let git_bin = locate_git().unwrap();
        let root = work.path().to_str().unwrap();
        let url = bare.path().to_string_lossy();
        assert_eq!(
            git_remote_add_with(&git_bin, root, "origin", &url)
                .unwrap()
                .remotes
                .len(),
            1
        );

        let res = git_remote_remove_with(&git_bin, root, "origin").unwrap();
        assert!(res.remotes.is_empty());
        let (code, _, _) = git(work.path(), &["config", "--get", "remote.origin.url"]);
        assert_ne!(code, 0, "删除后 remote.origin.url 不应再存在");
    }

    #[test]
    fn remote_remove_rejects_missing_remote() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());
        let git_bin = locate_git().unwrap();

        let err = git_remote_remove_with(
            &git_bin,
            work.path().to_str().unwrap(),
            "nope",
        )
        .unwrap_err();
        assert!(err.message.contains("远端不存在"), "actual: {}", err.message);
    }

    #[test]
    fn remote_remove_allows_current_upstream() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        let git_bin = locate_git().unwrap();
        let root = work2.path().to_str().unwrap();
        assert_eq!(
            git_remote(root).unwrap().current.as_deref(),
            Some("origin")
        );

        let res = git_remote_remove_with(&git_bin, root, "origin").unwrap();
        assert!(res.remotes.is_empty());
        assert_eq!(res.current, None);
    }

    #[test]
    fn remote_mutations_report_missing_git() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());
        let missing = Path::new("definitely-missing-git-binary-xyz");

        let err = git_remote_add_with(missing, work.path().to_str().unwrap(), "up", "https://x")
            .unwrap_err();
        assert!(err.message.contains("未检测到系统 git"), "actual: {}", err.message);
    }

    // ---------- 远程分支 ----------

    /// 在 work1 创建并推送 `name` 分支到 bare，随后切回 main
    fn push_remote_branch(work1: &Path, bare: &Path, name: &str) {
        assert_eq!(git(work1, &["checkout", "-b", name]).0, 0);
        std::fs::write(work1.join(format!("{name}.txt")), format!("{name}\n")).unwrap();
        assert_eq!(git(work1, &["add", "-A"]).0, 0);
        assert_eq!(git(work1, &["commit", "-m", name]).0, 0);
        assert_eq!(git(work1, &["push", bare.to_str().unwrap(), name]).0, 0);
        assert_eq!(git(work1, &["checkout", "main"]).0, 0);
    }

    #[test]
    fn split_remote_branch_validates_format() {
        assert!(split_remote_branch("origin/main").is_ok());
        assert!(split_remote_branch("main").is_err());
        assert!(split_remote_branch("origin/").is_err());
    }

    #[test]
    fn branches_lists_remote_branches_and_upstream() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        push_remote_branch(work1.path(), bare.path(), "dev");
        assert_eq!(git(work2.path(), &["fetch", "origin"]).0, 0);

        let bs = git_branch(work2.path().to_str().unwrap()).unwrap();
        assert!(bs.remote_branches.contains(&"origin/main".to_string()));
        assert!(bs.remote_branches.contains(&"origin/dev".to_string()));
        assert_eq!(bs.current_upstream.as_deref(), Some("origin/main"));
        assert!(bs.branches.iter().all(|b| b == "main"));
    }

    #[test]
    fn remote_fetch_refreshes_remote_branches() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        let git_bin = locate_git().unwrap();
        let root = work2.path().to_str().unwrap();
        assert!(!git_branch(root).unwrap().remote_branches.contains(&"origin/dev".to_string()));

        push_remote_branch(work1.path(), bare.path(), "dev");
        let res = git_fetch_with(&git_bin, root, "origin").unwrap();
        assert!(res.remote_branches.contains(&"origin/dev".to_string()));
        // remote 为空：取默认远端（origin）
        let res2 = git_fetch_with(&git_bin, root, "").unwrap();
        assert!(res2.remote_branches.contains(&"origin/dev".to_string()));
    }

    #[test]
    fn remote_fetch_reports_missing_git() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());
        let err = git_fetch_with(
            Path::new("definitely-missing-git-binary-xyz"),
            work.path().to_str().unwrap(),
            "origin",
        )
        .unwrap_err();
        assert!(err.message.contains("未检测到系统 git"), "actual: {}", err.message);
    }

    #[test]
    fn checkout_remote_creates_local_tracking_branch() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        push_remote_branch(work1.path(), bare.path(), "dev");
        assert_eq!(git(work2.path(), &["fetch", "origin"]).0, 0);
        let git_bin = locate_git().unwrap();

        let st = git_checkout_with(
            &git_bin,
            work2.path().to_str().unwrap(),
            "origin/dev",
        )
        .unwrap();
        assert_eq!(st.branch, "dev");
        let (code, remote, _) = git(work2.path(), &["config", "branch.dev.remote"]);
        assert_eq!(code, 0);
        assert_eq!(remote.trim(), "origin");
        let (_, merge, _) = git(work2.path(), &["config", "branch.dev.merge"]);
        assert_eq!(merge.trim(), "refs/heads/dev");
        let bs = git_branch(work2.path().to_str().unwrap()).unwrap();
        assert!(bs.branches.contains(&"dev".to_string()));
        assert_eq!(bs.current_upstream.as_deref(), Some("origin/dev"));
    }

    #[test]
    fn checkout_remote_rejects_existing_local_branch() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        push_remote_branch(work1.path(), bare.path(), "dev");
        assert_eq!(git(work2.path(), &["fetch", "origin"]).0, 0);
        assert_eq!(git(work2.path(), &["branch", "dev"]).0, 0);
        let git_bin = locate_git().unwrap();

        let err = git_checkout_with(
            &git_bin,
            work2.path().to_str().unwrap(),
            "origin/dev",
        )
        .unwrap_err();
        assert!(err.message.contains("已存在"), "actual: {}", err.message);
    }

    #[test]
    fn checkout_remote_rejects_missing_remote_branch() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        let git_bin = locate_git().unwrap();

        let err = git_checkout_with(
            &git_bin,
            work2.path().to_str().unwrap(),
            "origin/nope",
        )
        .unwrap_err();
        assert!(err.message.contains("远端分支不存在"), "actual: {}", err.message);
    }

    #[test]
    fn checkout_remote_reports_missing_git() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());
        let err = git_checkout_with(
            Path::new("definitely-missing-git-binary-xyz"),
            work.path().to_str().unwrap(),
            "origin/dev",
        )
        .unwrap_err();
        assert!(err.message.contains("未检测到系统 git"), "actual: {}", err.message);
    }

    #[test]
    fn delete_remote_branch_removes_ref() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        push_remote_branch(work1.path(), bare.path(), "dev");
        assert_eq!(git(work2.path(), &["fetch", "origin"]).0, 0);
        let git_bin = locate_git().unwrap();

        let res = git_push_delete_with(
            &git_bin,
            work2.path().to_str().unwrap(),
            "origin/dev",
        )
        .unwrap();
        assert!(!res.remote_branches.contains(&"origin/dev".to_string()));
        let (code, _, _) = git(bare.path(), &["rev-parse", "dev"]);
        assert_ne!(code, 0, "远端 dev 分支应已被删除");
    }

    #[test]
    fn delete_remote_branch_rejects_missing() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        let git_bin = locate_git().unwrap();

        let err = git_push_delete_with(
            &git_bin,
            work2.path().to_str().unwrap(),
            "origin/nope",
        )
        .unwrap_err();
        assert!(err.message.contains("远端分支不存在"), "actual: {}", err.message);
    }

    #[test]
    fn delete_remote_branch_reports_missing_git() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());
        let err = git_push_delete_with(
            Path::new("definitely-missing-git-binary-xyz"),
            work.path().to_str().unwrap(),
            "origin/main",
        )
        .unwrap_err();
        assert!(err.message.contains("未检测到系统 git"), "actual: {}", err.message);
    }

    #[test]
    fn status_reports_has_remote() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let work = TempDir::new().unwrap();
        std::fs::write(work.path().join("a.txt"), "x\n").unwrap();
        init_committed_repo(work.path());
        let st = git_status(work.path().to_str().unwrap()).unwrap();
        assert!(!st.has_remote);

        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        let st = git_status(work2.path().to_str().unwrap()).unwrap();
        assert!(st.has_remote);
    }

    #[test]
    fn switch_upstream_sets_branch_upstream() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare1 = TempDir::new().unwrap();
        let bare2 = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare1.path(), work1.path(), work2.path());
        // bare2 复制 work1（含 main），work2 添加 upstream 远端并拉取
        assert_eq!(
            git(work1.path(), &["clone", "--bare", ".", bare2.path().to_str().unwrap()]).0,
            0
        );
        let git_bin = locate_git().unwrap();
        let root = work2.path().to_str().unwrap();
        assert_eq!(
            git(work2.path(), &["remote", "add", "upstream", bare2.path().to_str().unwrap()]).0,
            0
        );
        assert_eq!(git(work2.path(), &["fetch", "upstream"]).0, 0);

        let res = git_set_upstream_with(&git_bin, root, "upstream").unwrap();
        assert_eq!(res.current.as_deref(), Some("upstream"));
        let (_, remote, _) = git(work2.path(), &["config", "branch.main.remote"]);
        assert_eq!(remote.trim(), "upstream");
        let (_, merge, _) = git(work2.path(), &["config", "branch.main.merge"]);
        assert_eq!(merge.trim(), "refs/heads/main");
    }

    #[test]
    fn switch_upstream_rejects_detached_head() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        assert_eq!(git(work2.path(), &["checkout", "--detach"]).0, 0);
        let git_bin = locate_git().unwrap();

        let err = git_set_upstream_with(
            &git_bin,
            work2.path().to_str().unwrap(),
            "origin",
        )
        .unwrap_err();
        assert!(err.message.contains("游离"), "actual: {}", err.message);
    }

    #[test]
    fn switch_upstream_rejects_missing_remote_branch() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare1 = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare1.path(), work1.path(), work2.path());
        assert_eq!(git(empty.path(), &["init", "--bare"]).0, 0);
        assert_eq!(
            git(work2.path(), &["remote", "add", "up", empty.path().to_str().unwrap()]).0,
            0
        );
        assert_eq!(git(work2.path(), &["fetch", "up"]).0, 0);
        let git_bin = locate_git().unwrap();

        let err = git_set_upstream_with(
            &git_bin,
            work2.path().to_str().unwrap(),
            "up",
        )
        .unwrap_err();
        assert!(
            err.message.contains("远端没有同名分支"),
            "actual: {}",
            err.message
        );
    }

    #[test]
    fn switch_upstream_rejects_missing_remote() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        let git_bin = locate_git().unwrap();

        let err = git_set_upstream_with(
            &git_bin,
            work2.path().to_str().unwrap(),
            "nope",
        )
        .unwrap_err();
        assert!(err.message.contains("不存在"), "actual: {}", err.message);
    }

    #[test]
    fn switch_upstream_reports_missing_git() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());

        let err = git_set_upstream_with(
            Path::new("definitely-missing-git-binary-xyz"),
            work2.path().to_str().unwrap(),
            "origin",
        )
        .unwrap_err();
        assert!(err.message.contains("未检测到系统 git"), "actual: {}", err.message);
    }

    // ---------- 分支合并 ----------

    /// main 提交 base 后切出 feature 分支并提交 feat（main 停在 base）
    fn setup_merge_repo(root: &Path) {
        std::fs::write(root.join("a.txt"), "base\n").unwrap();
        init_committed_repo(root);
        assert_eq!(git(root, &["checkout", "-b", "feature"]).0, 0);
        std::fs::write(root.join("b.txt"), "feat\n").unwrap();
        assert_eq!(git(root, &["add", "b.txt"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "feat"]).0, 0);
        assert_eq!(git(root, &["checkout", "main"]).0, 0);
    }

    #[test]
    fn merge_branch_fast_forwards_when_ahead() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        setup_merge_repo(root);

        let res = git_merge(root.to_str().unwrap(), "feature").unwrap();
        assert_eq!(res.kind, "fast_forward");
        assert!(res.status.files.is_empty());
        // feature 新增的文件进入工作区，main 快进到 feature 提交
        assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "feat\n");
        assert_eq!(git(root, &["log", "-1", "--format=%s"]).1.trim(), "feat");
    }

    #[test]
    fn merge_branch_creates_merge_commit_when_diverged() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        setup_merge_repo(root);
        // main 侧新增 c.txt 并提交 → 分叉
        std::fs::write(root.join("c.txt"), "main\n").unwrap();
        assert_eq!(git(root, &["add", "c.txt"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "main-side"]).0, 0);

        let res = git_merge(root.to_str().unwrap(), "feature").unwrap();
        assert_eq!(res.kind, "merged");
        assert!(res.status.files.is_empty());
        // 双方内容都在
        assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "feat\n");
        assert_eq!(std::fs::read_to_string(root.join("c.txt")).unwrap(), "main\n");
        // HEAD 为合并提交
        let head_subject = git(root, &["log", "-1", "--format=%s"]).1;
        assert!(head_subject.contains("Merge branch 'feature'"));
        // 合并后再次合并 → up to date
        let res = git_merge(root.to_str().unwrap(), "feature").unwrap();
        assert_eq!(res.kind, "up_to_date");
    }

    #[test]
    fn merge_branch_conflict_aborts_without_changes() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "base\n").unwrap();
        init_committed_repo(root);
        // side 分支修改 a.txt
        assert_eq!(git(root, &["checkout", "-b", "side"]).0, 0);
        std::fs::write(root.join("a.txt"), "side\n").unwrap();
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "side"]).0, 0);
        // main 分支也修改 a.txt → 合并冲突
        assert_eq!(git(root, &["checkout", "main"]).0, 0);
        std::fs::write(root.join("a.txt"), "main\n").unwrap();
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "main-side"]).0, 0);

        let before_head = git(root, &["rev-parse", "HEAD"]).1;
        let err = git_merge(root.to_str().unwrap(), "side").unwrap_err();
        assert!(err.message.contains("冲突"));
        assert!(err.message.contains("a.txt"));
        // 中止后工作区与 HEAD 不变
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "main\n");
        assert_eq!(git(root, &["rev-parse", "HEAD"]).1, before_head);
        assert_eq!(git(root, &["status", "--porcelain"]).1.trim(), "");
    }

    #[test]
    fn merge_branch_guards() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        setup_merge_repo(root);

        // 不能合并当前分支到自身
        let err = git_merge(root.to_str().unwrap(), "main").unwrap_err();
        assert!(err.message.contains("自身"));
        // 不存在的分支
        let err = git_merge(root.to_str().unwrap(), "nope").unwrap_err();
        assert!(err.message.contains("不存在"));
        // 游离 HEAD 无法合并
        assert_eq!(git(root, &["checkout", "--detach", "main"]).0, 0);
        let err = git_merge(root.to_str().unwrap(), "feature").unwrap_err();
        assert!(err.message.contains("游离 HEAD"));
        assert_eq!(git(root, &["checkout", "main"]).0, 0);
    }

    #[test]
    fn merge_branch_refuses_when_local_changes_overlap() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "base\n").unwrap();
        init_committed_repo(root);
        // feature 修改 a.txt 并提交；main 未提交修改同一文件 → 重叠拒绝
        assert_eq!(git(root, &["checkout", "-b", "feature"]).0, 0);
        std::fs::write(root.join("a.txt"), "feat\n").unwrap();
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "feat"]).0, 0);
        assert_eq!(git(root, &["checkout", "main"]).0, 0);
        std::fs::write(root.join("a.txt"), "dirty\n").unwrap();

        let err = git_merge(root.to_str().unwrap(), "feature").unwrap_err();
        assert!(err.message.contains("会被合并覆盖"));
        assert!(err.message.contains("a.txt"));
        // 拒绝后工作区与 HEAD 不变
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "dirty\n");
        assert_eq!(git(root, &["log", "-1", "--format=%s"]).1.trim(), "init");
    }

    // ---------- 提交历史 ----------

    #[test]
    fn commit_log_lists_commits_newest_first() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root); // init
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "second"]).0, 0);
        assert_eq!(git(root, &["commit", "--allow-empty", "-m", "third"]).0, 0);

        let log = git_log(root.to_str().unwrap(), 10, None).unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(log[0].subject, "third");
        assert_eq!(log[1].subject, "second");
        assert_eq!(log[2].subject, "init");
        assert_eq!(log[0].short_hash.len(), 7);
        assert_eq!(log[0].hash.len(), 40);
        assert!(!log[0].author.is_empty());
        assert!(log[0].time_secs > 0);
    }

    #[test]
    fn commit_log_respects_limit() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        for i in 0..5 {
            assert_eq!(git(root, &["commit", "--allow-empty", "-m", &format!("c{i}")]).0, 0);
        }
        let log = git_log(root.to_str().unwrap(), 3, None).unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(log[0].subject, "c4");
        assert_eq!(log[2].subject, "c2");
    }

    #[test]
    fn commit_log_empty_when_no_commits() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x\n").unwrap();
        let _ = git_init(root.to_str().unwrap()).unwrap();
        let log = git_log(root.to_str().unwrap(), 10, None).unwrap();
        assert!(log.is_empty());
    }

    #[test]
    fn commit_log_paginates_with_before() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        for i in 0..8 {
            assert_eq!(git(root, &["commit", "--allow-empty", "-m", &format!("c{i}")]).0, 0);
        }
        // 共 9 条：init、c0..c7（最新在前 c7..c0、init）
        let first = git_log(root.to_str().unwrap(), 3, None).unwrap();
        assert_eq!(first.len(), 3);
        assert_eq!(first[0].subject, "c7");
        assert_eq!(first[2].subject, "c5");
        let second =
            git_log(root.to_str().unwrap(), 3, Some(first[2].hash.clone())).unwrap();
        assert_eq!(second.len(), 3);
        assert_eq!(second[0].subject, "c4");
        assert_eq!(second[2].subject, "c2");
        let third =
            git_log(root.to_str().unwrap(), 3, Some(second[2].hash.clone())).unwrap();
        assert_eq!(third.len(), 3);
        assert_eq!(third[0].subject, "c1");
        assert_eq!(third[2].subject, "init");
        // 游标为最后一条时返回空
        let tail = git_log(root.to_str().unwrap(), 3, Some(third[2].hash.clone())).unwrap();
        assert!(tail.is_empty());
        // 无效游标返回空，避免前端重复追加
        let bogus = git_log(root.to_str().unwrap(), 3, Some("0".repeat(40))).unwrap();
        assert!(bogus.is_empty());
    }

    #[test]
    fn commit_detail_metadata_and_files() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        let first = git(root, &["rev-parse", "HEAD"]).1.trim().to_string();
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        std::fs::write(root.join("b.txt"), "new\n").unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "second\n\nbody line"]).0, 0);
        let second = git(root, &["rev-parse", "HEAD"]).1.trim().to_string();
        let detail = git_commit_detail(root.to_str().unwrap(), &second).unwrap();
        assert_eq!(detail.hash, second);
        assert_eq!(detail.short_hash.len(), 7);
        assert_eq!(detail.subject, "second");
        assert!(detail.body.contains("body line"));
        assert_eq!(detail.author, "t");
        assert_eq!(detail.author_email, "t@t");
        assert_eq!(detail.committer, "t");
        assert_eq!(detail.parents, vec![first]);
        let mut files: Vec<(String, FileStatus, u32, u32)> = detail
            .files
            .iter()
            .map(|f| {
                (
                    f.path.clone(),
                    f.status,
                    f.insertions,
                    f.deletions,
                )
            })
            .collect();
        files.sort();
        assert_eq!(
            files,
            vec![
                ("a.txt".to_string(), FileStatus::Modified, 1, 1),
                ("b.txt".to_string(), FileStatus::Added, 1, 0),
            ]
        );
    }

    #[test]
    fn commit_detail_root_commit_uses_empty_tree() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x\n").unwrap();
        init_committed_repo(root);
        let hash = git(root, &["rev-parse", "HEAD"]).1.trim().to_string();
        let detail = git_commit_detail(root.to_str().unwrap(), &hash).unwrap();
        assert!(detail.parents.is_empty());
        assert_eq!(detail.files.len(), 1);
        assert_eq!(detail.files[0].path, "a.txt");
        assert_eq!(detail.files[0].status, FileStatus::Added);
        assert_eq!(detail.files[0].insertions, 1);
        assert_eq!(detail.files[0].deletions, 0);
    }

    #[test]
    fn commit_detail_merge_uses_first_parent() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        // main 侧修改 a.txt
        std::fs::write(root.join("a.txt"), "main\n").unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "main change"]).0, 0);
        // side 分支新增 b.txt
        assert_eq!(git(root, &["checkout", "-b", "side"]).0, 0);
        std::fs::write(root.join("b.txt"), "side\n").unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "side change"]).0, 0);
        // 合并回 main（不同文件无冲突）
        assert_eq!(git(root, &["checkout", "main"]).0, 0);
        assert_eq!(
            git(root, &["merge", "--no-ff", "-m", "merge side", "side"]).0,
            0
        );
        let merge_hash = git(root, &["rev-parse", "HEAD"]).1.trim().to_string();
        let detail = git_commit_detail(root.to_str().unwrap(), &merge_hash).unwrap();
        assert_eq!(detail.parents.len(), 2);
        assert_eq!(detail.subject, "merge side");
        // 对比第一父提交（main）：仅 b.txt 新增
        assert_eq!(detail.files.len(), 1);
        assert_eq!(detail.files[0].path, "b.txt");
        assert_eq!(detail.files[0].status, FileStatus::Added);
    }

    #[test]
    fn commit_detail_rename_and_binary() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("old.txt"), "content\n").unwrap();
        std::fs::write(root.join("bin.dat"), [0u8, 1, 2, 3]).unwrap();
        init_committed_repo(root);
        assert_eq!(git(root, &["mv", "old.txt", "new.txt"]).0, 0);
        std::fs::write(root.join("bin.dat"), [9u8, 8, 7]).unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "rename+bin"]).0, 0);
        let hash = git(root, &["rev-parse", "HEAD"]).1.trim().to_string();
        let detail = git_commit_detail(root.to_str().unwrap(), &hash).unwrap();
        let renamed = detail
            .files
            .iter()
            .find(|f| f.status == FileStatus::Renamed)
            .expect("存在重命名文件");
        assert_eq!(renamed.path, "new.txt");
        assert!(!renamed.binary);
        let bin = detail
            .files
            .iter()
            .find(|f| f.path == "bin.dat")
            .expect("存在二进制文件");
        assert_eq!(bin.status, FileStatus::Modified);
        assert!(bin.binary);
        assert_eq!(bin.insertions, 0);
        assert_eq!(bin.deletions, 0);
    }

    #[test]
    fn commit_detail_empty_commit_has_empty_files() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x\n").unwrap();
        init_committed_repo(root);
        assert_eq!(git(root, &["commit", "--allow-empty", "-m", "empty"]).0, 0);
        let hash = git(root, &["rev-parse", "HEAD"]).1.trim().to_string();
        let detail = git_commit_detail(root.to_str().unwrap(), &hash).unwrap();
        assert!(detail.files.is_empty());
        assert_eq!(detail.subject, "empty");
    }

    #[test]
    fn commit_detail_invalid_hash_errors() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x\n").unwrap();
        init_committed_repo(root);
        let err = git_commit_detail(root.to_str().unwrap(), &"0".repeat(40)).unwrap_err();
        assert!(err.message.contains("无法读取提交详情"));
    }

    #[test]
    fn commit_file_diff_returns_rows() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        init_committed_repo(root);
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "second"]).0, 0);
        let hash = git(root, &["rev-parse", "HEAD"]).1.trim().to_string();
        let rows = git_commit_file_diff(root.to_str().unwrap(), &hash, "a.txt").unwrap();
        assert!(rows.iter().any(|r| matches!(r, DiffRow::Del { .. })));
        assert!(rows.iter().any(|r| matches!(r, DiffRow::Add { .. })));
        // 两树中都不存在的路径：返回空行
        assert!(
            git_commit_file_diff(root.to_str().unwrap(), &hash, "nope.txt")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn commit_file_diff_binary_errors() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("bin.dat"), [0u8, 1, 2]).unwrap();
        init_committed_repo(root);
        std::fs::write(root.join("bin.dat"), [3u8, 4, 5]).unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "bin change"]).0, 0);
        let hash = git(root, &["rev-parse", "HEAD"]).1.trim().to_string();
        let err = git_commit_file_diff(root.to_str().unwrap(), &hash, "bin.dat").unwrap_err();
        assert!(err.message.contains("二进制"));
    }

    #[test]
    fn commit_file_diff_with_late_nul_returns_rows() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let mut content = late_nul_content();
        std::fs::write(root.join("mix.dat"), &content).unwrap();
        init_committed_repo(root);
        let needle = b"AFTER-BEFORE";
        let pos = content.windows(needle.len()).position(|w| w == needle).unwrap();
        content.splice(pos..pos + needle.len(), b"AFTER-CHANGED".iter().copied());
        std::fs::write(root.join("mix.dat"), &content).unwrap();
        assert_eq!(git(root, &["add", "-A"]).0, 0);
        assert_eq!(git(root, &["commit", "-m", "change"]).0, 0);
        let hash = git(root, &["rev-parse", "HEAD"]).1.trim().to_string();
        let rows = git_commit_file_diff(root.to_str().unwrap(), &hash, "mix.dat").unwrap();
        assert!(
            rows.iter()
                .any(|r| matches!(r, DiffRow::Add { text, .. } if text.contains("AFTER-CHANGED")))
        );
    }
}
