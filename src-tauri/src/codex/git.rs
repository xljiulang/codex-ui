use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use gix::bstr::{BStr, BString, ByteSlice};
use gix::progress::Discard;
use gix::status::tree_index::TrackRenames;
use gix::status::UntrackedFiles;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::codex::path_util::{clean_path, norm_key};

/// 单文件大小上限（暂存/diff 等全量读入内存的操作），超过直接报错
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// 串行化所有 git 操作：gix 不像 git 那样写索引锁文件，并发写会互相覆盖。
/// 锁在 spawn_blocking 任务内部获取并持有到任务真正结束，超时后任务继续执行时
/// 后续操作也会排队等待，避免与后台仍在运行的 git 操作并发读写仓库。
static GIT_OP_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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

/// 变更状态（前端按小写字符串展示）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[derive(PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Untracked,
}

/// 变更文件条目
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFile {
    /// 相对仓库根的路径（正斜杠分隔）
    pub path: String,
    /// added | modified | deleted | untracked
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
    pub repo_root: String,
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

// ---------- 仓库打开 ----------

/// 打开（向上发现）Git 仓库；非仓库返回 `NotARepo`
fn open_repo(path: &str) -> Result<gix::Repository, GitError> {
    match gix::discover(path) {
        Ok(repo) => Ok(repo),
        Err(err) => {
            use gix::discover::upwards::Error as Up;
            let is_not_repo = matches!(
                err,
                gix::discover::Error::Discover(
                    Up::NoGitRepository { .. }
                        | Up::NoGitRepositoryWithinCeiling { .. }
                        | Up::NoGitRepositoryWithinFs { .. }
                        | Up::NoMatchingCeilingDir
                )
            );
            if is_not_repo {
                Err(not_a_repo())
            } else {
                Err(git_err(format!("无法打开 Git 仓库: {err}")))
            }
        }
    }
}

/// 相对路径字节 → 展示字符串
fn path_str(p: &BStr) -> String {
    p.to_str_lossy().into_owned()
}

/// 路径是否包含 node_modules 组件（任意深度，大小写不敏感）
fn has_node_modules_component(path: &str) -> bool {
    path.split('/').any(|c| c.eq_ignore_ascii_case("node_modules"))
}

// ---------- 状态 ----------

/// 合并同一路径的多个状态（暂存 + 未暂存），与 `git status` 的 XY 合并规则一致：
/// deleted > added > modified > untracked
fn merge_file(
    files: &mut Vec<GitFile>,
    index_of: &mut HashMap<String, usize>,
    path: String,
    status: FileStatus,
    staged: bool,
    worktree: bool,
) {
    let prio = |s: FileStatus| match s {
        FileStatus::Deleted => 4,
        FileStatus::Added => 3,
        FileStatus::Modified => 2,
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

fn status_sync(path: &str) -> Result<GitStatus, GitError> {
    let repo = open_repo(path)?;
    let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
    let branch = match repo.head_name() {
        Ok(Some(name)) => name.shorten().to_str_lossy().into_owned(),
        _ => "HEAD".to_string(),
    };

    let mut files: Vec<GitFile> = Vec::new();
    let mut index_of: HashMap<String, usize> = HashMap::new();

    let platform = repo
        .status(Discard)
        .map_err(|e| git_err(format!("初始化 git status 失败: {e}")))?
        .untracked_files(UntrackedFiles::Files)
        .index_worktree_rewrites(None)
        .tree_index_track_renames(TrackRenames::Disabled);

    let iter = platform
        .into_iter(Vec::<BString>::new())
        .map_err(|e| git_err(format!("git status 失败: {e}")))?;

    for item in iter {
        let item = item.map_err(|e| git_err(format!("git status 失败: {e}")))?;
        match item {
            // HEAD → 索引 的变化（暂存区）
            gix::status::Item::TreeIndex(change) => {
                use gix::diff::index::Change;
                match change {
                    Change::Addition { location, .. } => {
                        merge_file(
                            &mut files,
                            &mut index_of,
                            path_str(&location),
                            FileStatus::Added,
                            true,
                            false,
                        );
                    }
                    Change::Deletion { location, .. } => {
                        merge_file(
                            &mut files,
                            &mut index_of,
                            path_str(&location),
                            FileStatus::Deleted,
                            true,
                            false,
                        );
                    }
                    Change::Modification { location, .. } => {
                        merge_file(
                            &mut files,
                            &mut index_of,
                            path_str(&location),
                            FileStatus::Modified,
                            true,
                            false,
                        );
                    }
                    // 重命名检测已关闭，理论上不会出现
                    Change::Rewrite { .. } => continue,
                }
            }
            // 索引 → 工作区 的变化（含未跟踪文件）
            gix::status::Item::IndexWorktree(item) => {
                use gix::status::index_worktree::Item as IwItem;
                use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
                match item {
                    IwItem::Modification {
                        rela_path, status, ..
                    } => {
                        let status = match status {
                            // gix 版简化：冲突按“修改”展示
                            EntryStatus::Conflict { .. } => FileStatus::Modified,
                            EntryStatus::Change(Change::Removed) => FileStatus::Deleted,
                            EntryStatus::Change(_) => FileStatus::Modified,
                            EntryStatus::NeedsUpdate(_) => continue,
                            EntryStatus::IntentToAdd => FileStatus::Added,
                        };
                        merge_file(
                            &mut files,
                            &mut index_of,
                            path_str(rela_path.as_ref()),
                            status,
                            false,
                            true,
                        );
                    }
                    IwItem::DirectoryContents { entry, .. } => {
                        if matches!(entry.status, gix::dir::entry::Status::Untracked) {
                            merge_file(
                                &mut files,
                                &mut index_of,
                                path_str(entry.rela_path.as_ref()),
                                FileStatus::Untracked,
                                false,
                                true,
                            );
                        }
                    }
                    // 重命名检测已关闭，理论上不会出现
                    IwItem::Rewrite { .. } => continue,
                }
            }
        }
    }

    // 无条件忽略 node_modules（即使已被跟踪也隐藏）
    files.retain(|f| !has_node_modules_component(&f.path));
    files.sort_by_key(|a| a.path.to_lowercase());
    Ok(GitStatus {
        repo_root: clean_path(workdir),
        branch,
        has_remote: !repo.remote_names().is_empty(),
        files,
    })
}

fn init_sync(path: &str) -> Result<GitStatus, GitError> {
    match open_repo(path) {
        Ok(_) => return Err(git_err("当前目录已经是 Git 仓库，无需初始化")),
        Err(e) if e.code != GitErrorCode::NotARepo => return Err(e),
        Err(_) => {}
    }
    // 仅初始化（默认分支 main，HEAD 未出生，不自动提交）
    gix::init(path).map_err(|e| git_err(format!("git 初始化失败: {e}")))?;
    status_sync(path)
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

/// 取当前分支短名（游离 HEAD 或 HEAD 未出生时为 "HEAD"）
fn current_branch(repo: &gix::Repository) -> String {
    match repo.head_name() {
        Ok(Some(name)) => name.shorten().to_str_lossy().into_owned(),
        _ => "HEAD".to_string(),
    }
}

fn branches_sync(path: &str) -> Result<GitBranches, GitError> {
    let repo = open_repo(path)?;
    let current = current_branch(&repo);
    let mut branches: Vec<String> = Vec::new();
    let mut remote_branches: Vec<String> = Vec::new();
    let references = repo
        .references()
        .map_err(|e| git_err(format!("读取引用失败: {e}")))?;
    let iter = references
        .local_branches()
        .map_err(|e| git_err(format!("读取分支失败: {e}")))?;
    for item in iter {
        let item = item.map_err(|e| git_err(format!("读取分支失败: {e}")))?;
        branches.push(item.name().shorten().to_str_lossy().into_owned());
    }
    let remote_iter = references
        .remote_branches()
        .map_err(|e| git_err(format!("读取远程分支失败: {e}")))?;
    for item in remote_iter {
        let item = item.map_err(|e| git_err(format!("读取远程分支失败: {e}")))?;
        remote_branches.push(item.name().shorten().to_str_lossy().into_owned());
    }
    branches.sort_by_key(|a| a.to_lowercase());
    remote_branches.sort_by_key(|a| a.to_lowercase());
    let current_upstream = current_remote_branch(&repo, &current);
    Ok(GitBranches {
        current,
        branches,
        remote_branches,
        current_upstream,
    })
}

/// 当前分支上游的「远端/分支」短名（无上游/游离 HEAD/解析失败时为 None）
fn current_remote_branch(repo: &gix::Repository, branch: &str) -> Option<String> {
    if branch == "HEAD" {
        return None;
    }
    resolve_remote_target(repo, branch)
        .ok()
        .map(|(label, remote_branch)| format!("{label}/{remote_branch}"))
}

fn create_branch_sync(path: &str, name: &str) -> Result<GitStatus, GitError> {
    let name = name.trim();
    validate_branch_name(name)?;
    let repo = open_repo(path)?;
    let head_id = repo
        .head_id()
        .map_err(|_| git_err("仓库还没有任何提交，无法创建分支"))?;
    let full = gix::refs::Category::LocalBranch
        .to_full_name(BStr::new(name))
        .map_err(|e| git_err(format!("分支名不合法: {e}")))?;
    if repo.find_reference(&full).is_ok() {
        return Err(git_err(format!("分支 {name} 已存在")));
    }
    let head_obj = head_id
        .object()
        .map_err(|e| git_err(format!("读取 HEAD 失败: {e}")))?;
    repo.edit_reference(gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Update {
            log: Default::default(),
            expected: gix::refs::transaction::PreviousValue::MustNotExist,
            new: gix::refs::Target::Object(head_obj.id),
        },
        name: full,
        deref: false,
    })
    .map_err(|e| git_err(format!("创建分支失败: {e}")))?;
    status_sync(path)
}

fn delete_branch_sync(path: &str, name: &str) -> Result<GitStatus, GitError> {
    let name = name.trim();
    let repo = open_repo(path)?;
    if current_branch(&repo) == name {
        return Err(git_err("不能删除当前所在分支"));
    }
    let full = gix::refs::Category::LocalBranch
        .to_full_name(BStr::new(name))
        .map_err(|e| git_err(format!("分支名不合法: {e}")))?;
    let _ = repo
        .find_reference(&full)
        .map_err(|_| git_err(format!("分支 {name} 不存在")))?;
    repo.edit_reference(gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Delete {
            expected: gix::refs::transaction::PreviousValue::Any,
            log: gix::refs::transaction::RefLog::AndReference,
        },
        name: full,
        deref: false,
    })
    .map_err(|e| git_err(format!("删除分支失败: {e}")))?;
    status_sync(path)
}

/// 捕获当前 HEAD 的目标（符号引用或对象），用于写阶段失败时恢复。
/// `None` 表示 HEAD 原本未出生（新仓库无提交）。
fn capture_head_target(repo: &gix::Repository) -> Result<Option<gix::refs::Target>, GitError> {
    let head = repo.head().map_err(|e| git_err(format!("读取 HEAD 失败: {e}")))?;
    Ok(match head.kind {
        gix::head::Kind::Symbolic(r) => Some(gix::refs::Target::Symbolic(r.name)),
        gix::head::Kind::Detached { target, .. } => Some(gix::refs::Target::Object(target)),
        gix::head::Kind::Unborn(_) => None,
    })
}

/// 将 HEAD 恢复到捕获的目标；`None` 表示恢复“未出生”状态（删除 HEAD 引用）
fn restore_head(repo: &gix::Repository, target: &Option<gix::refs::Target>) -> Result<(), GitError> {
    let name: gix::refs::FullName = "HEAD".try_into().expect("valid");
    let change = match target {
        Some(gix::refs::Target::Symbolic(full)) => gix::refs::transaction::Change::Update {
            log: Default::default(),
            expected: gix::refs::transaction::PreviousValue::Any,
            new: gix::refs::Target::Symbolic(full.clone()),
        },
        Some(gix::refs::Target::Object(oid)) => gix::refs::transaction::Change::Update {
            log: Default::default(),
            expected: gix::refs::transaction::PreviousValue::Any,
            new: gix::refs::Target::Object(*oid),
        },
        None => gix::refs::transaction::Change::Delete {
            expected: gix::refs::transaction::PreviousValue::Any,
            log: gix::refs::transaction::RefLog::AndReference,
        },
    };
    repo.edit_reference(gix::refs::transaction::RefEdit {
        change,
        name,
        deref: false,
    })
    .map_err(|e| git_err(format!("恢复 HEAD 失败: {e}")))?;
    Ok(())
}

/// 将 `paths` 对应的工作区文件恢复为 `tree` 中的内容：
/// tree 有该路径 → 写回 blob；没有（纯新增）→ 删除文件。用于写阶段失败的尽最大努力回滚。
fn restore_paths_from_tree(
    workdir: &Path,
    tree: &gix::Tree,
    paths: &[String],
) -> Result<(), GitError> {
    for rel in paths {
        let dest = workdir.join(rel);
        let entry = tree
            .lookup_entry_by_path(Path::new(rel))
            .map_err(|e| git_err(format!("回滚时在旧树中查找 {rel} 失败: {e}")))?;
        match entry {
            Some(e) if e.mode().is_blob() => {
                let mut obj = e
                    .object()
                    .map_err(|err| git_err(format!("回滚时读取 {rel} 对象失败: {err}")))?;
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).map_err(|err| {
                        git_err(format!("回滚时创建目录失败 {}: {err}", clean_path(parent)))
                    })?;
                }
                std::fs::write(&dest, std::mem::take(&mut obj.data)).map_err(|err| {
                    git_err(format!("回滚时写回文件失败 {}: {err}", clean_path(&dest)))
                })?;
            }
            _ => {
                if dest.is_file() || dest.is_symlink() {
                    std::fs::remove_file(&dest).map_err(|err| {
                        git_err(format!("回滚时删除文件失败 {}: {err}", clean_path(&dest)))
                    })?;
                }
                remove_empty_parents(workdir, &dest);
            }
        }
    }
    Ok(())
}

fn switch_branch_sync(path: &str, name: &str) -> Result<GitStatus, GitError> {
    let name = name.trim();
    let repo = open_repo(path)?;
    let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
    let full = gix::refs::Category::LocalBranch
        .to_full_name(BStr::new(name))
        .map_err(|e| git_err(format!("分支名不合法: {e}")))?;
    let mut target_ref = repo
        .find_reference(&full)
        .map_err(|_| git_err(format!("分支 {name} 不存在")))?;
    if current_branch(&repo) == name {
        return Err(git_err(format!("当前已在分支 {name}")));
    }

    // 守卫 1：有已跟踪改动时拒绝切换，避免覆盖本地修改
    if repo
        .is_dirty()
        .map_err(|e| git_err(format!("检查工作区状态失败: {e}")))?
    {
        return Err(git_err("工作区有已跟踪的改动，请先提交或还原后再切换分支"));
    }

    let target_tree_id = target_ref
        .peel_to_id()
        .map_err(|e| git_err(format!("解析分支 {name} 失败: {e}")))?;
    let target_tree = target_tree_id
        .object()
        .map_err(|e| git_err(format!("读取分支 {name} 失败: {e}")))?
        .into_commit()
        .tree_id()
        .map_err(|e| git_err(format!("读取分支 {name} 失败: {e}")))?
        .object()
        .map_err(|e| git_err(format!("读取分支 {name} 失败: {e}")))?
        .into_tree();
    let old_tree_id = repo
        .head_tree_id_or_empty()
        .map_err(|e| git_err(format!("读取 HEAD 失败: {e}")))?;
    let old_tree = old_tree_id
        .object()
        .map_err(|e| git_err(format!("读取 HEAD 树失败: {e}")))?
        .into_tree();

    let changes = repo
        .diff_tree_to_tree(
            Some(&old_tree),
            Some(&target_tree),
            gix::diff::Options::default().with_rewrites(None),
        )
        .map_err(|e| git_err(format!("计算分支差异失败: {e}")))?;

    // 守卫 2：目标路径已存在但未被跟踪（未跟踪文件将被覆盖）
    let old_index = repo
        .index_or_empty()
        .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
    for change in &changes {
        let location = match change {
            gix::diff::tree_with_rewrites::Change::Addition { location, .. }
            | gix::diff::tree_with_rewrites::Change::Modification { location, .. } => location,
            _ => continue,
        };
        // 目录级（tree）条目在索引中没有对应条目，且真实碰撞由文件级变化逐条校验，跳过
        if change.entry_mode().is_tree() {
            continue;
        }
        if workdir.join(path_str(location.as_ref())).exists()
            && old_index.entry_by_path(location.as_ref()).is_none()
        {
            return Err(git_err(format!("未跟踪文件将被覆盖: {}", path_str(location.as_ref()))));
        }
    }

    // 写阶段前置状态捕获（用于失败回滚）
    let mut old_index_owned = old_index.into_owned_or_cloned();
    let old_head = capture_head_target(&repo)?;
    let changed_paths: Vec<String> = changes
        .iter()
        .filter(|c| !c.entry_mode().is_tree())
        .filter_map(|c| match c {
            gix::diff::tree_with_rewrites::Change::Addition { location, .. }
            | gix::diff::tree_with_rewrites::Change::Modification { location, .. }
            | gix::diff::tree_with_rewrites::Change::Deletion { location, .. } => {
                Some(path_str(location.as_ref()))
            }
            gix::diff::tree_with_rewrites::Change::Rewrite { .. } => None,
        })
        .collect();

    // 写阶段（工作区 → 索引 → HEAD）。任一步失败都回滚到写前状态，避免半切换。
    let write_result = (|| -> Result<(), GitError> {
        // ① 同步工作区文件
        for change in &changes {
            match change {
                gix::diff::tree_with_rewrites::Change::Addition { location, id, .. }
                | gix::diff::tree_with_rewrites::Change::Modification { location, id, .. } => {
                    let rel = path_str(location.as_ref());
                    let dest = workdir.join(&rel);
                    let obj = repo
                        .find_object(*id)
                        .map_err(|e| git_err(format!("读取对象失败: {e}")))?;
                    if obj.kind != gix::objs::Kind::Blob {
                        continue;
                    }
                    let mut obj = obj;
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            git_err(format!("创建目录失败 {}: {e}", clean_path(parent)))
                        })?;
                    }
                    std::fs::write(&dest, std::mem::take(&mut obj.data))
                        .map_err(|e| git_err(format!("写入文件失败 {}: {e}", clean_path(&dest))))?;
                }
                gix::diff::tree_with_rewrites::Change::Deletion { location, .. } => {
                    let p = workdir.join(path_str(location.as_ref()));
                    if p.is_file() || p.is_symlink() {
                        std::fs::remove_file(&p)
                            .map_err(|e| git_err(format!("删除文件失败 {}: {e}", clean_path(&p))))?;
                    }
                    remove_empty_parents(workdir, &p);
                }
                gix::diff::tree_with_rewrites::Change::Rewrite { .. } => continue,
            }
        }

        // ② 重建索引
        let mut new_index = repo
            .index_from_tree(&target_tree.id)
            .map_err(|e| git_err(format!("重建索引失败: {e}")))?;
        new_index
            .write(gix::index::write::Options::default())
            .map_err(|e| git_err(format!("写入索引失败: {e}")))?;

        // ③ 更新 HEAD 指向目标分支
        repo.edit_reference(gix::refs::transaction::RefEdit {
            change: gix::refs::transaction::Change::Update {
                log: Default::default(),
                expected: gix::refs::transaction::PreviousValue::Any,
                new: gix::refs::Target::Symbolic(full),
            },
            name: "HEAD".try_into().expect("valid"),
            deref: false,
        })
        .map_err(|e| git_err(format!("切换分支失败: {e}")))?;
        Ok(())
    })();

    if let Err(e) = write_result {
        // 尽最大努力回滚：工作区文件 → 旧索引 → 旧 HEAD
        let _ = restore_paths_from_tree(workdir, &old_tree, &changed_paths);
        let _ = old_index_owned.write(gix::index::write::Options::default());
        let _ = restore_head(&repo, &old_head);
        return Err(e);
    }

    status_sync(path)
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

/// 从索引移除指定路径的所有条目（含冲突各阶段）并写回；无对应条目时不写文件
fn remove_index_entry(repo: &gix::Repository, rel: &str) -> Result<(), GitError> {
    let index = repo
        .index_or_empty()
        .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
    let mut index = index.into_owned_or_cloned();
    let rel_b = BStr::new(rel.as_bytes());
    let before = index.entries().len();
    index.remove_entries(|_, p, _| p == rel_b);
    if index.entries().len() != before {
        index.remove_tree();
        index.remove_resolve_undo();
        index
            .write(gix::index::write::Options::default())
            .map_err(|e| git_err(format!("写入索引失败: {e}")))?;
    }
    Ok(())
}

/// 取 HEAD 树中 `path` 的条目（blob id + mode）；HEAD 未出生或无此路径 → None
fn head_entry(
    repo: &gix::Repository,
    path: &str,
) -> Result<Option<(gix::hash::ObjectId, gix::index::entry::Mode)>, GitError> {
    let tree_id = repo
        .head_tree_id_or_empty()
        .map_err(|e| git_err(format!("读取 HEAD 失败: {e}")))?;
    let tree = tree_id
        .object()
        .map_err(|e| git_err(format!("读取 HEAD 树失败: {e}")))?
        .into_tree();
    let Some(entry) = tree
        .lookup_entry_by_path(path)
        .map_err(|e| git_err(format!("在 HEAD 中查找 {path} 失败: {e}")))?
    else {
        return Ok(None);
    };
    Ok(Some((
        entry.object_id(),
        gix::index::entry::Mode::from(entry.mode().kind()),
    )))
}

/// 将索引中多个路径重置为 HEAD 状态（等价 `git restore --staged <paths>`）：
/// HEAD 有该路径 → 索引条目更新为 HEAD 的 blob/mode；HEAD 无 → 移除条目。不写盘。
fn reset_index_entries(
    index: &mut gix::index::File,
    repo: &gix::Repository,
    rels: &[String],
) -> Result<(), GitError> {
    for rel in rels {
        let head = head_entry(repo, rel)?;
        let rel_b = BStr::new(rel.as_bytes());
        index.remove_entries(|_, p, _| p == rel_b);
        if let Some((id, mode)) = head {
            index.dangerously_push_entry(
                gix::index::entry::Stat::default(),
                id,
                gix::index::entry::Flags::empty(),
                mode,
                rel_b,
            );
        }
    }
    index.sort_entries();
    index.remove_tree();
    index.remove_resolve_undo();
    Ok(())
}

/// 单文件取消暂存：索引条目重置为 HEAD 状态，工作区不变
fn reset_index_entry(repo: &gix::Repository, rel: &str) -> Result<(), GitError> {
    let index = repo
        .index_or_empty()
        .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
    let mut index = index.into_owned_or_cloned();
    let rels = [rel.to_string()];
    reset_index_entries(&mut index, repo, &rels)?;
    index
        .write(gix::index::write::Options::default())
        .map_err(|e| git_err(format!("写入索引失败: {e}")))?;
    Ok(())
}

/// HEAD 树中是否存在 `path`（未出生 HEAD 视为不存在）
fn head_has_path(repo: &gix::Repository, path: &str) -> Result<bool, GitError> {
    Ok(head_entry(repo, path)?.is_some())
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

/// 将单个文件登记进索引（不写盘）：工作区文件存在 → 写入 blob 并登记/更新索引条目；
/// 已删除 → 移除条目（暂存删除）。与 `git add` 语义一致：先移除旧条目（含冲突各阶段）。
fn stage_index_entry(
    index: &mut gix::index::File,
    repo: &gix::Repository,
    workdir: &Path,
    rel: &str,
) -> Result<(), GitError> {
    let abs = workdir.join(rel);
    let rel_b = BStr::new(rel.as_bytes());
    index.remove_entries(|_, p, _| p == rel_b);

    match std::fs::symlink_metadata(&abs) {
        Ok(meta) if !meta.is_dir() => {
            if meta.len() > MAX_FILE_BYTES {
                return Err(git_err(format!(
                    "文件过大（超过 {} MB），无法暂存: {}",
                    MAX_FILE_BYTES / (1024 * 1024),
                    clean_path(&abs)
                )));
            }
            let data = std::fs::read(&abs)
                .map_err(|e| git_err(format!("读取文件失败 {}: {e}", clean_path(&abs))))?;
            let id = repo
                .write_blob(&data)
                .map_err(|e| git_err(format!("写入对象失败: {e}")))?;
            let stat = gix::index::fs::Metadata::from_path_no_follow(&abs)
                .ok()
                .and_then(|m| gix::index::entry::Stat::from_fs(&m).ok())
                .unwrap_or_default();
            let mode = if meta.file_type().is_symlink() {
                gix::index::entry::Mode::SYMLINK
            } else {
                gix::index::entry::Mode::FILE
            };
            index.dangerously_push_entry(
                stat,
                id.detach(),
                gix::index::entry::Flags::empty(),
                mode,
                rel_b,
            );
            index.sort_entries();
            index.remove_tree();
            index.remove_resolve_undo();
        }
        Ok(_) => return Err(git_err("暂存目标不是文件")),
        Err(_) => {
            // 工作区文件已不存在：暂存删除，索引条目已在上方移除
            index.remove_tree();
            index.remove_resolve_undo();
        }
    }
    Ok(())
}

/// 单文件暂存（等价 `git add <path>`）
fn stage_file_sync(path: &str, rel: &str) -> Result<(), GitError> {
    let repo = open_repo(path)?;
    let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
    let rel = validate_rel_path(rel)?;
    let index = repo
        .index_or_empty()
        .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
    let mut index = index.into_owned_or_cloned();
    stage_index_entry(&mut index, &repo, workdir, &rel)?;
    index
        .write(gix::index::write::Options::default())
        .map_err(|e| git_err(format!("写入索引失败: {e}")))?;
    Ok(())
}

/// 暂存：支持文件或目录；目录 = 递归暂存其下所有变更文件（未跟踪即“添加跟踪”），
/// 目录内已删除的已跟踪文件同样暂存删除
fn stage_sync(path: &str, rel: &str) -> Result<GitStatus, GitError> {
    let repo = open_repo(path)?;
    let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
    let rel = validate_rel_path(rel)?;
    if workdir.join(&rel).is_dir() {
        let st = status_sync(path)?;
        let rels: Vec<String> = files_under(&st.files, &rel)
            .into_iter()
            .map(|f| f.path)
            .collect();
        let index = repo
            .index_or_empty()
            .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
        let mut index = index.into_owned_or_cloned();
        for r in &rels {
            let r = validate_rel_path(r)?;
            stage_index_entry(&mut index, &repo, workdir, &r)?;
        }
        index
            .write(gix::index::write::Options::default())
            .map_err(|e| git_err(format!("写入索引失败: {e}")))?;
    } else {
        stage_file_sync(path, &rel)?;
    }
    status_sync(path)
}

/// 单文件取消暂存：索引条目重置为 HEAD 状态，工作区不变
fn unstage_file_sync(path: &str, rel: &str) -> Result<(), GitError> {
    let repo = open_repo(path)?;
    let rel = validate_rel_path(rel)?;
    reset_index_entry(&repo, &rel)
}

/// 取消暂存：支持文件或目录；目录 = 其下所有变更文件索引重置为 HEAD
fn unstage_sync(path: &str, rel: &str) -> Result<GitStatus, GitError> {
    let repo = open_repo(path)?;
    let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
    let rel = validate_rel_path(rel)?;
    if workdir.join(&rel).is_dir() {
        let st = status_sync(path)?;
        let rels: Vec<String> = files_under(&st.files, &rel)
            .into_iter()
            .map(|f| f.path)
            .collect();
        let index = repo
            .index_or_empty()
            .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
        let mut index = index.into_owned_or_cloned();
        reset_index_entries(&mut index, &repo, &rels)?;
        index
            .write(gix::index::write::Options::default())
            .map_err(|e| git_err(format!("写入索引失败: {e}")))?;
    } else {
        unstage_file_sync(path, &rel)?;
    }
    status_sync(path)
}

/// 全部暂存：等价 `git add -A`，将当前所有工作区侧变更暂存
/// （含未跟踪、删除与冲突文件，冲突按当前工作区内容暂存即标记已解决）
fn stage_all_sync(path: &str) -> Result<GitStatus, GitError> {
    let st = status_sync(path)?;
    let repo = open_repo(path)?;
    let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
    let index = repo
        .index_or_empty()
        .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
    let mut index = index.into_owned_or_cloned();
    for f in st.files.iter().filter(|f| f.worktree) {
        let r = validate_rel_path(&f.path)?;
        stage_index_entry(&mut index, &repo, workdir, &r)?;
    }
    index
        .write(gix::index::write::Options::default())
        .map_err(|e| git_err(format!("写入索引失败: {e}")))?;
    status_sync(path)
}

/// 全部取消暂存：所有已暂存文件索引重置为 HEAD（HEAD 无则移除），工作区不变
fn unstage_all_sync(path: &str) -> Result<GitStatus, GitError> {
    let st = status_sync(path)?;
    let repo = open_repo(path)?;
    let index = repo
        .index_or_empty()
        .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
    let mut index = index.into_owned_or_cloned();
    let rels: Vec<String> = st
        .files
        .iter()
        .filter(|f| f.staged)
        .map(|f| f.path.clone())
        .collect();
    reset_index_entries(&mut index, &repo, &rels)?;
    index
        .write(gix::index::write::Options::default())
        .map_err(|e| git_err(format!("写入索引失败: {e}")))?;
    status_sync(path)
}

/// 单文件工作区还原：HEAD 有该路径 → 写回 HEAD 内容；HEAD 无 → 删除工作区文件。不写索引。
fn restore_file_worktree(repo: &gix::Repository, workdir: &Path, rel: &str) -> Result<(), GitError> {
    let abs = workdir.join(rel);
    if head_has_path(repo, rel)? {
        let data = head_blob(repo, rel)?;
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| git_err(format!("创建目录失败 {}: {e}", clean_path(parent))))?;
        }
        std::fs::write(&abs, data)
            .map_err(|e| git_err(format!("写入文件失败 {}: {e}", clean_path(&abs))))?;
    } else if abs.is_file() || abs.is_symlink() {
        std::fs::remove_file(&abs)
            .map_err(|e| git_err(format!("删除文件失败 {}: {e}", clean_path(&abs))))?;
        remove_empty_parents(workdir, &abs);
    }
    Ok(())
}

/// 单文件还原：完全丢弃该文件本地更改——先取消暂存，工作区恢复为 HEAD 内容；
/// HEAD 无此文件（未跟踪/新增）则删除工作区文件
fn restore_file_sync(path: &str, rel: &str) -> Result<(), GitError> {
    let repo = open_repo(path)?;
    let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
    let rel = validate_rel_path(rel)?;
    reset_index_entry(&repo, &rel)?;
    restore_file_worktree(&repo, workdir, &rel)
}

/// 还原：支持文件或目录；目录 = 其下所有变更文件丢弃（已跟踪恢复 HEAD，未跟踪删除）
fn restore_sync(path: &str, rel: &str) -> Result<GitStatus, GitError> {
    let repo = open_repo(path)?;
    let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
    let rel = validate_rel_path(rel)?;
    if workdir.join(&rel).is_dir() {
        let st = status_sync(path)?;
        let rels: Vec<String> = files_under(&st.files, &rel)
            .into_iter()
            .map(|f| f.path)
            .collect();
        for r in &rels {
            restore_file_worktree(&repo, workdir, r)?;
        }
        let index = repo
            .index_or_empty()
            .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
        let mut index = index.into_owned_or_cloned();
        reset_index_entries(&mut index, &repo, &rels)?;
        index
            .write(gix::index::write::Options::default())
            .map_err(|e| git_err(format!("写入索引失败: {e}")))?;
    } else {
        restore_file_sync(path, &rel)?;
    }
    status_sync(path)
}

/// 删除文件：移除工作区文件并从索引移除（等价 `git rm`），逐级清理空目录；不支持目录
fn delete_sync(path: &str, rel: &str) -> Result<GitStatus, GitError> {
    let repo = open_repo(path)?;
    let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
    let rel = validate_rel_path(rel)?;
    let abs = workdir.join(&rel);
    if abs.is_dir() {
        return Err(git_err("不支持删除目录"));
    }
    if abs.is_file() || abs.is_symlink() {
        std::fs::remove_file(&abs)
            .map_err(|e| git_err(format!("删除文件失败 {}: {e}", clean_path(&abs))))?;
        remove_empty_parents(workdir, &abs);
    }
    remove_index_entry(&repo, &rel)?;
    status_sync(path)
}

/// 添加到 .gitignore：文件追加 `/路径`，目录追加 `/路径/`（不存在则创建，幂等跳过重复条目）
fn ignore_sync(path: &str, rel: &str) -> Result<GitStatus, GitError> {
    let repo = open_repo(path)?;
    let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
    let rel = validate_rel_path(rel)?;
    let pattern = if workdir.join(&rel).is_dir() {
        format!("/{}/", rel.trim_end_matches('/'))
    } else {
        format!("/{rel}")
    };
    let plain = pattern.trim_start_matches('/');
    let gitignore = workdir.join(".gitignore");
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
    status_sync(path)
}

// ---------- 提交与拉取 ----------

/// 从索引全量条目构建树（等价 `git write-tree`）：以空树为起点逐条 upsert。
/// 提交树由索引决定，未暂存的工作区改动不会进入树。
fn index_tree_id(repo: &gix::Repository) -> Result<gix::hash::ObjectId, GitError> {
    use gix::object::tree::{Editor, EntryKind};
    let index = repo
        .index_or_empty()
        .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
    let empty_tree = repo
        .find_object(gix::hash::ObjectId::empty_tree(repo.object_hash()))
        .map_err(|e| git_err(format!("读取空树失败: {e}")))?
        .into_tree();
    let mut editor = Editor::new(&empty_tree)
        .map_err(|e| git_err(format!("初始化树编辑器失败: {e}")))?;
    for entry in index.entries() {
        let kind = match entry.mode {
            gix::index::entry::Mode::FILE => EntryKind::Blob,
            gix::index::entry::Mode::FILE_EXECUTABLE => EntryKind::BlobExecutable,
            gix::index::entry::Mode::SYMLINK => EntryKind::Link,
            gix::index::entry::Mode::COMMIT => EntryKind::Commit,
            gix::index::entry::Mode::DIR => continue,
            _ => continue,
        };
        editor
            .upsert(entry.path(&index), kind, entry.id)
            .map_err(|e| git_err(format!("构建提交树失败: {e}")))?;
    }
    editor
        .write()
        .map(|id| id.detach())
        .map_err(|e| git_err(format!("写入提交树失败: {e}")))
}

/// 提交已暂存更改（等价 `git commit`）：索引全量条目构建树，创建提交并更新 HEAD，
/// 成功后重建索引为提交树。未暂存更改保留在工作区。
fn commit_sync(path: &str, message: &str) -> Result<GitStatus, GitError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(git_err("提交消息不能为空"));
    }
    let repo = open_repo(path)?;
    // 冲突检测：索引存在 stage != 0（unmerged）条目时拒绝提交，避免用未定义行为建树
    let index = repo
        .index_or_empty()
        .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
    if index.entries().iter().any(|e| e.stage_raw() != 0) {
        return Err(git_err("仓库存在未解决的合并冲突，请先解决冲突后再提交"));
    }
    let st = status_sync(path)?;
    if !st.files.iter().any(|f| f.staged) {
        return Err(git_err("没有已暂存的更改，请先暂存文件"));
    }
    if repo.author().is_none() || repo.committer().is_none() {
        return Err(git_err("未配置 Git 用户信息（user.name / user.email），请先在仓库配置后重试"));
    }
    let tree_id = index_tree_id(&repo)?;
    let parents: Vec<gix::hash::ObjectId> = match repo.head_id() {
        Ok(id) => vec![id.detach()],
        Err(_) => Vec::new(),
    };
    repo.commit("HEAD", message, tree_id, parents)
        .map_err(|e| git_err(format!("提交失败: {e}")))?;
    // 提交后索引重置为提交树（等价 git commit 后的索引状态）
    let mut index = repo
        .index_from_tree(&tree_id)
        .map_err(|e| git_err(format!("重建索引失败: {e}")))?;
    index
        .write(gix::index::write::Options::default())
        .map_err(|e| git_err(format!("写入索引失败: {e}")))?;
    status_sync(path)
}

/// 从 HEAD 沿第一父链遍历提交历史（最多 `limit` 条，最新在前）；
/// 合并提交会顺带展开其其它父提交。仓库尚无提交时返回空列表。
/// `before` 为续页游标（上一批最后一条的完整 hash）：命中前只遍历不输出，
/// 命中后继续输出下一批，保证顺序与首次加载一致；游标无效或不可达时返回空列表。
fn commit_log_sync(
    path: &str,
    limit: usize,
    before: Option<String>,
) -> Result<Vec<GitCommitEntry>, GitError> {
    let repo = open_repo(path)?;
    let limit = limit.clamp(1, 200);
    let before = before
        .as_deref()
        .and_then(|s| gix::hash::ObjectId::from_hex(s.as_bytes()).ok());
    let mut out: Vec<GitCommitEntry> = Vec::new();
    let mut stack: Vec<gix::hash::ObjectId> = match repo.head_id() {
        Ok(id) => vec![id.detach()],
        Err(_) => return Ok(out),
    };
    let mut seen: std::collections::HashSet<gix::hash::ObjectId> = std::collections::HashSet::new();
    let mut reached = before.is_none();
    while let Some(id) = stack.pop() {
        if out.len() >= limit {
            break;
        }
        if !seen.insert(id) {
            continue;
        }
        let commit = repo
            .find_object(id)
            .map_err(|e| git_err(format!("读取提交失败: {e}")))?
            .into_commit();
        // 先处理第一父（栈逆序压入），近似 git log 的第一父优先顺序
        let parents: Vec<gix::hash::ObjectId> = commit.parent_ids().map(|p| p.detach()).collect();
        if !reached {
            if before.as_ref() == Some(&id) {
                reached = true;
            }
            for p in parents.into_iter().rev() {
                stack.push(p);
            }
            continue;
        }
        let subject = commit
            .message()
            .map(|m| m.summary().to_str_lossy().into_owned())
            .unwrap_or_default();
        let author = commit
            .author()
            .map(|a| a.name.to_str_lossy().into_owned())
            .unwrap_or_default();
        let time_secs = commit.time().map(|t| t.seconds).unwrap_or(0);
        out.push(GitCommitEntry {
            hash: id.to_string(),
            short_hash: id.to_string().chars().take(7).collect(),
            subject,
            author,
            time_secs,
        });
        for p in parents.into_iter().rev() {
            stack.push(p);
        }
    }
    Ok(out)
}

/// 计算 旧树 → 新树 的变更路径（add/mod/delete；跳过 rewrite 与目录条目）
fn diff_changed_paths(
    repo: &gix::Repository,
    old_tree_id: &gix::hash::oid,
    new_tree_id: &gix::hash::oid,
) -> Result<Vec<String>, GitError> {
    let old_tree = repo
        .find_object(old_tree_id)
        .map_err(|e| git_err(format!("读取旧树失败: {e}")))?
        .into_tree();
    let new_tree = repo
        .find_object(new_tree_id)
        .map_err(|e| git_err(format!("读取新树失败: {e}")))?
        .into_tree();
    let changes = repo
        .diff_tree_to_tree(
            Some(&old_tree),
            Some(&new_tree),
            gix::diff::Options::default().with_rewrites(None),
        )
        .map_err(|e| git_err(format!("计算树差异失败: {e}")))?;
    let mut paths = Vec::new();
    for change in &changes {
        if change.entry_mode().is_tree() {
            continue;
        }
        let location = match change {
            gix::diff::tree_with_rewrites::Change::Addition { location, .. }
            | gix::diff::tree_with_rewrites::Change::Modification { location, .. }
            | gix::diff::tree_with_rewrites::Change::Deletion { location, .. } => location,
            gix::diff::tree_with_rewrites::Change::Rewrite { .. } => continue,
        };
        paths.push(path_str(location.as_ref()));
    }
    Ok(paths)
}

/// 把 旧树 → 新树 的变更同步到工作区文件（写/删），不重建索引；
/// 目标路径已存在但未被跟踪时拒绝，避免覆盖未跟踪文件。
fn sync_worktree_files(
    repo: &gix::Repository,
    workdir: &Path,
    old_tree_id: &gix::hash::oid,
    new_tree_id: &gix::hash::oid,
) -> Result<(), GitError> {
    let old_tree = repo
        .find_object(old_tree_id)
        .map_err(|e| git_err(format!("读取旧树失败: {e}")))?
        .into_tree();
    let new_tree = repo
        .find_object(new_tree_id)
        .map_err(|e| git_err(format!("读取新树失败: {e}")))?
        .into_tree();
    let changes = repo
        .diff_tree_to_tree(
            Some(&old_tree),
            Some(&new_tree),
            gix::diff::Options::default().with_rewrites(None),
        )
        .map_err(|e| git_err(format!("计算树差异失败: {e}")))?;

    // 守卫：目标路径已存在但未被跟踪（未跟踪文件将被覆盖）
    let old_index = repo
        .index_or_empty()
        .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
    for change in &changes {
        let location = match change {
            gix::diff::tree_with_rewrites::Change::Addition { location, .. }
            | gix::diff::tree_with_rewrites::Change::Modification { location, .. } => location,
            _ => continue,
        };
        if change.entry_mode().is_tree() {
            continue;
        }
        if workdir.join(path_str(location.as_ref())).exists()
            && old_index.entry_by_path(location.as_ref()).is_none()
        {
            return Err(git_err(format!(
                "未跟踪文件将被覆盖: {}",
                path_str(location.as_ref())
            )));
        }
    }

    for change in &changes {
        match change {
            gix::diff::tree_with_rewrites::Change::Addition { location, id, .. }
            | gix::diff::tree_with_rewrites::Change::Modification { location, id, .. } => {
                let rel = path_str(location.as_ref());
                let dest = workdir.join(&rel);
                let obj = repo
                    .find_object(*id)
                    .map_err(|e| git_err(format!("读取对象失败: {e}")))?;
                if obj.kind != gix::objs::Kind::Blob {
                    continue;
                }
                let mut obj = obj;
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| git_err(format!("创建目录失败 {}: {e}", clean_path(parent))))?;
                }
                std::fs::write(&dest, std::mem::take(&mut obj.data))
                    .map_err(|e| git_err(format!("写入文件失败 {}: {e}", clean_path(&dest))))?;
            }
            gix::diff::tree_with_rewrites::Change::Deletion { location, .. } => {
                let p = workdir.join(path_str(location.as_ref()));
                if p.is_file() || p.is_symlink() {
                    std::fs::remove_file(&p)
                        .map_err(|e| git_err(format!("删除文件失败 {}: {e}", clean_path(&p))))?;
                }
                remove_empty_parents(workdir, &p);
            }
            gix::diff::tree_with_rewrites::Change::Rewrite { .. } => continue,
        }
    }
    Ok(())
}

/// 重建索引为 `new_tree_id`，并保留未被拉取触达的已暂存条目：
/// 有条目则重新登记旧 blob/mode，已暂存删除则保持移除。
fn rebuild_index_preserving_staged(
    repo: &gix::Repository,
    new_tree_id: &gix::hash::oid,
    staged_paths: &[String],
    pull_changed: &[String],
) -> Result<(), GitError> {
    let old_index = repo
        .index_or_empty()
        .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
    let mut new_index = repo
        .index_from_tree(new_tree_id)
        .map_err(|e| git_err(format!("重建索引失败: {e}")))?;
    for path in staged_paths {
        // 被拉取触达的已暂存路径已在冲突判定阶段拒绝，这里防御性跳过
        if pull_changed.iter().any(|p| p == path) {
            continue;
        }
        let rel_b = BStr::new(path.as_bytes());
        new_index.remove_entries(|_, p, _| p == rel_b);
        if let Some(entry) = old_index.entry_by_path(rel_b) {
            new_index.dangerously_push_entry(entry.stat, entry.id, entry.flags, entry.mode, rel_b);
        }
    }
    new_index.sort_entries();
    new_index.remove_tree();
    new_index.remove_resolve_undo();
    new_index
        .write(gix::index::write::Options::default())
        .map_err(|e| git_err(format!("写入索引失败: {e}")))?;
    Ok(())
}

/// 更新本地分支引用指向 `target`（用于快进）
fn update_branch_ref(
    repo: &gix::Repository,
    branch: &str,
    target: gix::hash::ObjectId,
) -> Result<(), GitError> {
    let full = gix::refs::Category::LocalBranch
        .to_full_name(BStr::new(branch))
        .map_err(|e| git_err(format!("分支名不合法: {e}")))?;
    repo.edit_reference(gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Update {
            log: Default::default(),
            expected: gix::refs::transaction::PreviousValue::Any,
            new: gix::refs::Target::Object(target),
        },
        name: full,
        deref: false,
    })
    .map_err(|e| git_err(format!("更新分支失败: {e}")))?;
    Ok(())
}

/// 提取冲突路径（ours/theirs 任一非 Rewrite 侧的位置）
fn conflict_path(c: &gix::merge::tree::Conflict) -> String {
    use gix::diff::tree_with_rewrites::Change;
    for change in [&c.ours, &c.theirs] {
        let loc = match change {
            Change::Addition { location, .. }
            | Change::Modification { location, .. }
            | Change::Deletion { location, .. } => location,
            Change::Rewrite { .. } => continue,
        };
        return path_str(loc.as_ref());
    }
    "<未知路径>".to_string()
}

/// 创建系统 git 命令：Windows GUI 应用下设置 CREATE_NO_WINDOW，
/// 避免拉取/推送子进程闪现黑色控制台窗口。
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

/// 拉取：调用系统 git（快进优先，`--ff-only`）。分叉时不自动合并，
/// 报错提示先经 GitView 分支合并手动合并，绝不留下冲突状态。
fn pull_sync(root: &str) -> Result<GitPullResult, GitError> {
    let git_bin = find_git()
        .ok_or_else(|| git_err("未检测到系统 git，请先安装 Git（git-scm.com）或将其加入 PATH"))?;
    pull_sync_with_git(&git_bin, root)
}

/// pull 实现（git 可执行文件路径可注入，便于测试“git 缺失”分支）
fn pull_sync_with_git(git_bin: &Path, root: &str) -> Result<GitPullResult, GitError> {
    let repo = open_repo(root)?;
    let branch = current_branch(&repo);
    if branch == "HEAD" {
        return Err(git_err("游离 HEAD 状态无法拉取，请先切换到某个分支"));
    }
    let (remote_label, remote_branch) = resolve_remote_target(&repo, &branch)?;
    let out = git_command(git_bin)
        .arg("-C")
        .arg(root)
        .args([
            "pull",
            "--no-rebase",
            "--ff-only",
            &remote_label,
            &remote_branch,
        ])
        // 固定英文输出便于解析；GIT_TERMINAL_PROMPT=0 避免在无终端环境挂起
        //（Git Credential Manager 等 GUI 助手不受影响，认证失败会快速报错）。
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                git_err("未检测到系统 git，请先安装 Git（git-scm.com）或将其加入 PATH")
            } else {
                git_err(format!("执行 git pull 失败: {e}"))
            }
        })?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{stdout}\n{stderr}");
    if !out.status.success() {
        return Err(git_err(pull_failure_message(&combined)));
    }
    let status = status_sync(root)?;
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
/// 探测系统 git 可执行文件：优先 PATH，回退常见安装目录。
/// 拉取/推送是仅有的两个依赖系统 git 的功能（其余读取/状态/提交/合并仍由 gix 完成）。
fn find_git() -> Option<PathBuf> {
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

/// 从远端跟踪 ref 解析远端分支名（refs/remotes/<remote>/<rb> 或 refs/heads/<rb>）
fn remote_branch_from_tracking(tracking: &gix::refs::FullName, remote_label: &str) -> Option<String> {
    let s = tracking.as_bstr().to_str_lossy();
    let prefix = format!("refs/remotes/{remote_label}/");
    if let Some(rest) = s.strip_prefix(&prefix) {
        return Some(rest.to_string());
    }
    if let Some(rest) = s.strip_prefix("refs/heads/") {
        return Some(rest.to_string());
    }
    None
}

/// 解析远端目标（远端名/URL + 远端分支名）：优先已配置的上游，
/// 无配置时以默认远端（origin）同名分支兜底。
fn resolve_remote_target(
    repo: &gix::Repository,
    branch: &str,
) -> Result<(String, String), GitError> {
    let branch_full = gix::refs::Category::LocalBranch
        .to_full_name(BStr::new(branch))
        .map_err(|e| git_err(format!("分支名不合法: {e}")))?;
    if let Some(Ok(tracking)) = repo
        .branch_remote_tracking_ref_name(branch_full.as_ref(), gix::remote::Direction::Fetch)
    {
        if let Some(remote) = repo.branch_remote(branch, gix::remote::Direction::Fetch) {
            let remote = remote.map_err(|e| git_err(format!("读取远端失败: {e}")))?;
            let remote_label = remote
                .name()
                .map(|n| n.as_bstr().to_str_lossy().into_owned())
                .or_else(|| {
                    remote
                        .url(gix::remote::Direction::Push)
                        .map(|u| u.to_bstring().to_string())
                })
                .ok_or_else(|| git_err("远端没有名称或地址"))?;
            let remote_branch = remote_branch_from_tracking(&tracking, &remote_label)
                .ok_or_else(|| git_err("无法解析上游远端分支名"))?;
            return Ok((remote_label, remote_branch));
        }
    }
    let remote = repo
        .find_fetch_remote(None)
        .map_err(|e| git_err(format!("未找到可用的远端: {e}")))?;
    let remote_label = remote
        .name()
        .map(|n| n.as_bstr().to_str_lossy().into_owned())
        .or_else(|| {
            remote
                .url(gix::remote::Direction::Push)
                .map(|u| u.to_bstring().to_string())
        })
        .ok_or_else(|| git_err("远端没有名称或地址"))?;
    Ok((remote_label, branch.to_string()))
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

/// 推送当前分支到上游远端（调用系统 git；首次推送自动设置上游 -u）。
fn push_sync(root: &str) -> Result<GitPushResult, GitError> {
    let git_bin = find_git()
        .ok_or_else(|| git_err("未检测到系统 git，请先安装 Git（git-scm.com）或将其加入 PATH"))?;
    push_sync_with_git(&git_bin, root)
}

/// push 实现（git 可执行文件路径可注入，便于测试“git 缺失”分支）
fn push_sync_with_git(git_bin: &Path, root: &str) -> Result<GitPushResult, GitError> {
    let repo = open_repo(root)?;
    let branch = current_branch(&repo);
    if branch == "HEAD" {
        return Err(git_err("游离 HEAD 状态无法推送，请先切换到某个分支"));
    }
    let (remote_label, remote_branch) = resolve_remote_target(&repo, &branch)?;
    let refspec = format!("{branch}:{remote_branch}");
    let out = git_command(git_bin)
        .arg("-C")
        .arg(root)
        .args(["push", "-u", &remote_label, &refspec])
        // 固定英文输出便于解析；GIT_TERMINAL_PROMPT=0 避免在无终端环境挂起
        //（Git Credential Manager 等 GUI 助手不受影响，认证失败会快速报错）。
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                git_err("未检测到系统 git，请先安装 Git（git-scm.com）或将其加入 PATH")
            } else {
                git_err(format!("执行 git push 失败: {e}"))
            }
        })?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{stdout}\n{stderr}");
    if !out.status.success() {
        return Err(git_err(push_failure_message(&combined)));
    }
    let status = status_sync(root)?;
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

// ---------- 远端管理 ----------

/// 远端列表（只读，gix 实现，不依赖系统 git）：名称排序，
/// fetch/push 地址按 gix 语义取（pushUrl 未配置时回退为拉取地址）。
fn remotes_sync(path: &str) -> Result<GitRemotes, GitError> {
    let repo = open_repo(path)?;
    let mut remotes: Vec<GitRemote> = Vec::new();
    for name in repo.remote_names() {
        let name = name.to_string();
        let remote = repo
            .find_remote(&name)
            .map_err(|e| git_err(format!("读取远端 {name} 失败: {e}")))?;
        remotes.push(GitRemote {
            fetch_url: remote
                .url(gix::remote::Direction::Fetch)
                .map(|u| u.to_bstring().to_string()),
            push_url: remote
                .url(gix::remote::Direction::Push)
                .map(|u| u.to_bstring().to_string()),
            name,
        });
    }
    remotes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(GitRemotes {
        current: current_remote_label(&repo),
        remotes,
    })
}

/// 当前分支的远端名：经 resolve_remote_target 解析（无远端/游离 HEAD/解析失败均为 None）
fn current_remote_label(repo: &gix::Repository) -> Option<String> {
    let branch = current_branch(repo);
    if branch == "HEAD" {
        return None;
    }
    resolve_remote_target(repo, &branch)
        .ok()
        .map(|(label, _)| label)
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

/// 执行 git 子命令（远端/网络类操作；锁由外层 run_blocking 统一持有），
/// 失败输出经 `map_error` 映射为中文提示。
fn run_git_mapped(
    git_bin: &Path,
    root: &str,
    args: &[&str],
    action: &str,
    map_error: impl Fn(&str) -> String,
) -> Result<(), GitError> {
    let out = git_command(git_bin)
        .arg("-C")
        .arg(root)
        .args(args)
        // 固定英文输出便于解析；GIT_TERMINAL_PROMPT=0 避免在无终端环境挂起
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                git_err("未检测到系统 git，请先安装 Git（git-scm.com）或将其加入 PATH")
            } else {
                git_err(format!("执行 {action} 失败: {e}"))
            }
        })?;
    if !out.status.success() {
        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        return Err(git_err(map_error(&combined)));
    }
    Ok(())
}

/// 执行 git remote 写操作（错误映射见 remote_failure_message）
fn run_remote_git(git_bin: &Path, root: &str, args: &[&str]) -> Result<(), GitError> {
    run_git_mapped(git_bin, root, args, "git remote", remote_failure_message)
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

/// 添加远端（调用系统 git：`git remote add <name> <url>`，自动生成默认 fetch refspec）
fn remote_add_sync_with_git(
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
    run_remote_git(git_bin, root, &["remote", "add", name, url])?;
    remotes_sync(root)
}

/// 修改远端拉取地址（`git remote set-url <name> <url>`；pushUrl 保持不变）
fn remote_set_url_sync_with_git(
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
    run_remote_git(git_bin, root, &["remote", "set-url", name, url])?;
    remotes_sync(root)
}

/// 删除远端（`git remote remove <name>`；删除当前上游远端允许，由前端确认提示）
fn remote_remove_sync_with_git(git_bin: &Path, root: &str, name: &str) -> Result<GitRemotes, GitError> {
    let name = name.trim();
    validate_remote_name(name)?;
    run_remote_git(git_bin, root, &["remote", "remove", name])?;
    remotes_sync(root)
}

fn remote_add_sync(root: &str, name: &str, url: &str) -> Result<GitRemotes, GitError> {
    let git_bin = find_git()
        .ok_or_else(|| git_err("未检测到系统 git，请先安装 Git（git-scm.com）或将其加入 PATH"))?;
    remote_add_sync_with_git(&git_bin, root, name, url)
}

fn remote_set_url_sync(root: &str, name: &str, url: &str) -> Result<GitRemotes, GitError> {
    let git_bin = find_git()
        .ok_or_else(|| git_err("未检测到系统 git，请先安装 Git（git-scm.com）或将其加入 PATH"))?;
    remote_set_url_sync_with_git(&git_bin, root, name, url)
}

fn remote_remove_sync(root: &str, name: &str) -> Result<GitRemotes, GitError> {
    let git_bin = find_git()
        .ok_or_else(|| git_err("未检测到系统 git，请先安装 Git（git-scm.com）或将其加入 PATH"))?;
    remote_remove_sync_with_git(&git_bin, root, name)
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
fn remote_fetch_sync_with_git(
    git_bin: &Path,
    root: &str,
    remote: &str,
) -> Result<GitBranches, GitError> {
    let repo = open_repo(root)?;
    let trimmed = remote.trim();
    let remote_label = if trimmed.is_empty() {
        repo.remote_default_name(gix::remote::Direction::Fetch)
            .map(|n| n.to_string())
            .or_else(|| {
                repo.find_fetch_remote(None)
                    .ok()
                    .and_then(|r| r.name().map(|n| n.as_bstr().to_str_lossy().into_owned()))
            })
            .ok_or_else(|| git_err("未找到可用的远端"))?
    } else {
        trimmed.to_string()
    };
    run_git_mapped(
        git_bin,
        root,
        &["fetch", &remote_label],
        "git fetch",
        fetch_failure_message,
    )?;
    branches_sync(root)
}

/// 从远程分支检出为同名本地跟踪分支（`git checkout -b <branch> --track <remote>/<branch>`）。
/// 本地已有同名分支时拒绝（由前端路由到本地切换）。
fn branch_checkout_remote_sync_with_git(
    git_bin: &Path,
    root: &str,
    remote_branch: &str,
) -> Result<GitStatus, GitError> {
    let (remote_label, branch) = split_remote_branch(remote_branch)?;
    validate_remote_name(remote_label)?;
    validate_branch_name(branch)?;
    let repo = open_repo(root)?;
    let full = gix::refs::Category::LocalBranch
        .to_full_name(BStr::new(branch))
        .map_err(|e| git_err(format!("分支名不合法: {e}")))?;
    if repo.find_reference(&full).is_ok() {
        return Err(git_err(format!("本地分支 {branch} 已存在，请先切换到该分支")));
    }
    let track = format!("{remote_label}/{branch}");
    run_git_mapped(
        git_bin,
        root,
        &["checkout", "-b", branch, "--track", &track],
        "git checkout",
        remote_branch_failure_message,
    )?;
    status_sync(root)
}

/// 删除远程分支（`git push <remote> --delete <branch>`），返回刷新后的分支列表
fn remote_branch_delete_sync_with_git(
    git_bin: &Path,
    root: &str,
    remote_branch: &str,
) -> Result<GitBranches, GitError> {
    let (remote_label, branch) = split_remote_branch(remote_branch)?;
    validate_remote_name(remote_label)?;
    validate_branch_name(branch)?;
    run_git_mapped(
        git_bin,
        root,
        &["push", remote_label, "--delete", branch],
        "git push",
        remote_branch_failure_message,
    )?;
    branches_sync(root)
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
fn remote_switch_upstream_sync_with_git(
    git_bin: &Path,
    root: &str,
    remote: &str,
) -> Result<GitRemotes, GitError> {
    let remote = remote.trim();
    validate_remote_name(remote)?;
    let repo = open_repo(root)?;
    let _ = repo
        .find_remote(remote)
        .map_err(|_| git_err(format!("远端 {remote} 不存在")))?;
    let branch = current_branch(&repo);
    if branch == "HEAD" {
        return Err(git_err("游离 HEAD 状态无法设置上游，请先切换到某个分支"));
    }
    let target = format!("{remote}/{branch}");
    run_git_mapped(
        git_bin,
        root,
        &["branch", "--set-upstream-to", &target],
        "git branch",
        switch_upstream_failure_message,
    )?;
    remotes_sync(root)
}

fn remote_fetch_sync(root: &str, remote: &str) -> Result<GitBranches, GitError> {
    let git_bin = find_git()
        .ok_or_else(|| git_err("未检测到系统 git，请先安装 Git（git-scm.com）或将其加入 PATH"))?;
    remote_fetch_sync_with_git(&git_bin, root, remote)
}

fn branch_checkout_remote_sync(root: &str, remote_branch: &str) -> Result<GitStatus, GitError> {
    let git_bin = find_git()
        .ok_or_else(|| git_err("未检测到系统 git，请先安装 Git（git-scm.com）或将其加入 PATH"))?;
    branch_checkout_remote_sync_with_git(&git_bin, root, remote_branch)
}

fn remote_branch_delete_sync(root: &str, remote_branch: &str) -> Result<GitBranches, GitError> {
    let git_bin = find_git()
        .ok_or_else(|| git_err("未检测到系统 git，请先安装 Git（git-scm.com）或将其加入 PATH"))?;
    remote_branch_delete_sync_with_git(&git_bin, root, remote_branch)
}

fn remote_switch_upstream_sync(root: &str, remote: &str) -> Result<GitRemotes, GitError> {
    let git_bin = find_git()
        .ok_or_else(|| git_err("未检测到系统 git，请先安装 Git（git-scm.com）或将其加入 PATH"))?;
    remote_switch_upstream_sync_with_git(&git_bin, root, remote)
}

/// 将本地分支 `name` 合并到当前分支（等价 `git merge <name>`）：
/// 快进或三路合并（创建合并提交）。与拉取相同，仅当合并触达的路径与本地
/// 已暂存/未暂存改动重叠时拒绝（列文件）；提交级冲突时中止，不改动工作区与索引。
fn merge_branch_sync(path: &str, name: &str) -> Result<GitMergeResult, GitError> {
    let name = name.trim();
    let repo = open_repo(path)?;
    let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
    let branch = current_branch(&repo);
    if branch == "HEAD" {
        return Err(git_err("游离 HEAD 状态无法合并，请先切换到某个分支"));
    }
    if branch == name {
        return Err(git_err("不能将当前分支合并到自身"));
    }
    let full = gix::refs::Category::LocalBranch
        .to_full_name(BStr::new(name))
        .map_err(|e| git_err(format!("分支名不合法: {e}")))?;
    let source_id = repo
        .find_reference(&full)
        .map_err(|_| git_err(format!("分支 {name} 不存在")))?
        .peel_to_id()
        .map_err(|e| git_err(format!("解析分支 {name} 失败: {e}")))?;
    let source_oid = source_id.detach();
    let source_tree_oid = source_id
        .object()
        .map_err(|e| git_err(format!("读取分支 {name} 失败: {e}")))?
        .into_commit()
        .tree_id()
        .map_err(|e| git_err(format!("读取分支 {name} 失败: {e}")))?
        .detach();

    // 本地风险路径：已暂存（HEAD→索引）+ 已跟踪的工作区修改；
    // 未跟踪文件不纳入（由 sync_worktree_files 的覆盖守卫保护）。
    let st = status_sync(path)?;
    let staged_paths: Vec<String> = st
        .files
        .iter()
        .filter(|f| f.staged)
        .map(|f| f.path.clone())
        .collect();
    let local_risk_paths: Vec<String> = st
        .files
        .iter()
        .filter(|f| f.worktree && f.status != FileStatus::Untracked)
        .map(|f| f.path.clone())
        .collect();

    // 本地状态
    let local_oid = repo.head_id().ok().map(|id| id.detach());
    let local_tree_id = repo
        .head_tree_id_or_empty()
        .map_err(|e| git_err(format!("读取 HEAD 失败: {e}")))?;
    let local_tree_oid = local_tree_id.detach();

    // 判定合并类型与目标树（只读计算，不写入）
    let mut target_tree_oid: Option<gix::hash::ObjectId> = None;
    let (kind, message) = match local_oid {
        None => return Err(git_err("仓库还没有任何提交，无法合并")),
        Some(local) if local == source_oid => {
            ("up_to_date".to_string(), "分支已是最新，无需合并".to_string())
        }
        Some(local) => {
            let labels = gix::merge::blob::builtin_driver::text::Labels {
                ancestor: Some(BStr::new("ancestor")),
                current: Some(BStr::new("ours")),
                other: Some(BStr::new("theirs")),
            };
            let options = repo
                .tree_merge_options()
                .map_err(|e| git_err(format!("初始化合并选项失败: {e}")))?
                .into();
            let mut outcome = repo
                .merge_commits(local, source_oid, labels, options)
                .map_err(|e| git_err(format!("合并分支 {name} 失败: {e}")))?;
            let merge_bases: Vec<gix::hash::ObjectId> = outcome
                .merge_bases
                .iter()
                .flat_map(|nb| nb.iter().copied())
                .collect();
            if merge_bases.contains(&source_oid) {
                // 源分支是本地祖先 → 本地已包含其全部提交
                ("up_to_date".to_string(), "分支已是最新，无需合并".to_string())
            } else if merge_bases.contains(&local) {
                // 本地是源分支祖先 → 快进
                target_tree_oid = Some(source_tree_oid);
                (
                    "fast_forward".to_string(),
                    format!("已将分支 {name} 快进合并到 {branch}"),
                )
            } else {
                // 分叉：真实合并，提交级冲突则中止
                if !outcome.tree_merge.conflicts.is_empty() {
                    let paths: Vec<String> = outcome
                        .tree_merge
                        .conflicts
                        .iter()
                        .map(conflict_path)
                        .collect();
                    return Err(git_err(format!(
                        "合并 {name} 存在冲突，已中止（未修改任何文件）。冲突文件：{}",
                        paths.join("、")
                    )));
                }
                let merged_tree_oid = outcome
                    .tree_merge
                    .tree
                    .write()
                    .map(|id| id.detach())
                    .map_err(|e| git_err(format!("写入合并树失败: {e}")))?;
                target_tree_oid = Some(merged_tree_oid);
                (
                    "merged".to_string(),
                    format!("已将分支 {name} 合并到 {branch}"),
                )
            }
        }
    };

    // 冲突判定：合并触达的路径与本地风险路径重叠 → 在任何写入之前拒绝
    let merge_changed: Vec<String> = if let Some(target) = target_tree_oid {
        let changed = diff_changed_paths(&repo, &local_tree_oid, &target)?;
        let mut overlap: Vec<&String> = local_risk_paths
            .iter()
            .chain(staged_paths.iter())
            .filter(|p| changed.iter().any(|c| c == *p))
            .collect();
        overlap.sort();
        overlap.dedup();
        if !overlap.is_empty() {
            let names: Vec<&str> = overlap.iter().map(|s| s.as_str()).collect();
            return Err(git_err(format!(
                "本地更改会被合并覆盖，请先提交或还原后再合并：{}",
                names.join("、")
            )));
        }
        changed
    } else {
        Vec::new()
    };

    // 写阶段前置状态捕获（用于失败回滚）
    let old_index_owned = repo
        .index_or_empty()
        .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
    let mut old_index_owned = old_index_owned.into_owned_or_cloned();
    let local_tree = local_tree_id
        .object()
        .map_err(|e| git_err(format!("读取本地树失败: {e}")))?
        .into_tree();

    // 应用：同步工作区 → 重建索引 → 更新分支/创建合并提交。
    // 任一步失败都回滚到写前状态，避免留下半合并状态。
    let apply_result = (|| -> Result<(), GitError> {
        if let Some(target) = target_tree_oid {
            sync_worktree_files(&repo, workdir, &local_tree_oid, &target)?;
            rebuild_index_preserving_staged(&repo, &target, &staged_paths, &merge_changed)?;
        }
        match kind.as_str() {
            "fast_forward" => update_branch_ref(&repo, &branch, source_oid)?,
            "merged" => {
                let merge_msg = format!("Merge branch '{name}' into {branch}");
                let local_oid = *local_oid.as_ref().expect("合并必有本地提交");
                repo.commit(
                    "HEAD",
                    merge_msg,
                    target_tree_oid.expect("合并必有目标树"),
                    vec![local_oid, source_oid],
                )
                .map_err(|e| git_err(format!("创建合并提交失败: {e}")))?;
            }
            _ => {}
        }
        Ok(())
    })();

    if let Err(e) = apply_result {
        // 尽最大努力回滚：工作区 → 旧索引 → 分支引用回到本地提交
        let _ = restore_paths_from_tree(workdir, &local_tree, &merge_changed);
        let _ = old_index_owned.write(gix::index::write::Options::default());
        let branch_full = gix::refs::Category::LocalBranch
            .to_full_name(BStr::new(&branch))
            .map_err(|e| git_err(format!("分支名不合法: {e}")))?;
        let restore_change = match &local_oid {
            Some(oid) => gix::refs::transaction::Change::Update {
                log: Default::default(),
                expected: gix::refs::transaction::PreviousValue::Any,
                new: gix::refs::Target::Object(*oid),
            },
            None => gix::refs::transaction::Change::Delete {
                expected: gix::refs::transaction::PreviousValue::Any,
                log: gix::refs::transaction::RefLog::AndReference,
            },
        };
        let _ = repo.edit_reference(gix::refs::transaction::RefEdit {
            change: restore_change,
            name: branch_full,
            deref: false,
        });
        return Err(e);
    }

    Ok(GitMergeResult {
        status: status_sync(path)?,
        kind,
        message,
    })
}

// ---------- diff ----------

/// git 近似二进制判定：前 8000 字节内出现 NUL
fn looks_binary(data: &[u8]) -> bool {
    data[..data.len().min(8000)].contains(&0)
}

/// 渲染 unified diff（`@@ -a,b +c,d @@` + `+`/`-`/` ` 行，与 git diff -U3 兼容）；
/// 任一输入为二进制时返回 None
fn render_diff(old: &[u8], new: &[u8]) -> Option<String> {
    if looks_binary(old) || looks_binary(new) {
        return None;
    }
    use gix::diff::blob::unified_diff::{ConsumeBinaryHunk, ContextSize};
    use gix::diff::blob::{
        Algorithm, InternedInput, UnifiedDiff, diff_with_slider_heuristics,
        platform::resource::ByteLinesWithoutTerminator,
    };
    let input = InternedInput::new(
        ByteLinesWithoutTerminator::new(old),
        ByteLinesWithoutTerminator::new(new),
    );
    let diff = diff_with_slider_heuristics(Algorithm::default(), &input);
    let out: BString = UnifiedDiff::new(
        &diff,
        &input,
        ConsumeBinaryHunk::new(BString::default(), "\n"),
        ContextSize::symmetrical(3),
    )
    .consume()
    .ok()?;
    Some(out.to_str_lossy().into_owned())
}

/// 取 HEAD 树中 `path` 对应的 blob 内容（HEAD 未出生或无此路径 → 空）
fn head_blob(repo: &gix::Repository, path: &str) -> Result<Vec<u8>, GitError> {
    let tree_id = repo
        .head_tree_id_or_empty()
        .map_err(|e| git_err(format!("读取 HEAD 失败: {e}")))?;
    let tree = tree_id
        .object()
        .map_err(|e| git_err(format!("读取 HEAD 树失败: {e}")))?
        .into_tree();
    let Some(entry) = tree
        .lookup_entry_by_path(path)
        .map_err(|e| git_err(format!("在 HEAD 中查找 {path} 失败: {e}")))?
    else {
        return Ok(Vec::new());
    };
    let mut obj = entry
        .object()
        .map_err(|e| git_err(format!("读取 {path} 对象失败: {e}")))?;
    if obj.kind == gix::objs::Kind::Blob {
        let data = std::mem::take(&mut obj.data);
        if data.len() as u64 > MAX_FILE_BYTES {
            return Err(git_err(format!(
                "文件过大（超过 {} MB），无法处理: {path}",
                MAX_FILE_BYTES / (1024 * 1024)
            )));
        }
        Ok(data)
    } else {
        Ok(Vec::new())
    }
}

/// diff 基准统一为 HEAD（暂存 + 未暂存合并）；未跟踪文件为空 → 工作区内容。
/// 返回空串表示无内容变化。
fn diff_sync(root: &str, path: &str, _kind: &str) -> Result<String, GitError> {
    let path = validate_rel_path(path)?;
    let repo = open_repo(root)?;
    let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
    let old = head_blob(&repo, &path)?;
    // 已删除或不可读的文件视为空内容
    let new = std::fs::read(workdir.join(&path)).unwrap_or_default();
    if new.len() as u64 > MAX_FILE_BYTES {
        return Err(git_err(format!(
            "文件过大（超过 {} MB），无法显示差异: {path}",
            MAX_FILE_BYTES / (1024 * 1024)
        )));
    }
    if old == new {
        return Ok(String::new());
    }
    render_diff(&old, &new).ok_or_else(|| git_err("该文件是二进制文件，无法显示文本差异"))
}

/// 在阻塞线程中执行同步 git 操作，带指定超时（秒）
async fn run_blocking_with_timeout<T: Send + 'static>(
    timeout_secs: u64,
    f: impl FnOnce() -> Result<T, GitError> + Send + 'static,
) -> Result<T, GitError> {
    let task = tokio::task::spawn_blocking(move || {
        // 锁在任务内部持有，超时后任务仍在后台执行时，后续 git 操作会排队等待，
        // 避免与未结束的操作并发读写同一仓库。
        let _guard = git_op_lock().lock().unwrap_or_else(|e| e.into_inner());
        f()
    });
    match tokio::time::timeout(Duration::from_secs(timeout_secs), task).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => Err(git_err(format!("git 任务异常: {e}"))),
        Err(_) => Err(git_err(format!("git 操作超时（{timeout_secs} 秒）"))),
    }
}

/// 在阻塞线程中执行同步 git 操作，带 60s 超时
async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, GitError> + Send + 'static,
) -> Result<T, GitError> {
    run_blocking_with_timeout(60, f).await
}

#[tauri::command]
pub async fn git_changes_status(path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || status_sync(&path)).await
}

#[tauri::command]
pub async fn git_changes_commit(root: String, message: String) -> Result<GitStatus, GitError> {
    run_blocking(move || commit_sync(&root, &message)).await
}

#[tauri::command]
pub async fn git_changes_pull(root: String) -> Result<GitPullResult, GitError> {
    // 拉取涉及网络传输，放宽超时到 10 分钟
    run_blocking_with_timeout(600, move || pull_sync(&root)).await
}

#[tauri::command]
pub async fn git_changes_push(root: String) -> Result<GitPushResult, GitError> {
    // 推送涉及网络传输与可能的凭证交互，放宽超时到 10 分钟
    run_blocking_with_timeout(600, move || push_sync(&root)).await
}

/// 远端列表（只读，gix 实现，不依赖系统 git）
#[tauri::command]
pub async fn git_changes_remotes(path: String) -> Result<GitRemotes, GitError> {
    run_blocking(move || remotes_sync(&path)).await
}

/// 添加远端（调用系统 git）
#[tauri::command]
pub async fn git_changes_remote_add(root: String, name: String, url: String) -> Result<GitRemotes, GitError> {
    run_blocking(move || remote_add_sync(&root, &name, &url)).await
}

/// 修改远端拉取地址（调用系统 git）
#[tauri::command]
pub async fn git_changes_remote_set_url(
    root: String,
    name: String,
    url: String,
) -> Result<GitRemotes, GitError> {
    run_blocking(move || remote_set_url_sync(&root, &name, &url)).await
}

/// 删除远端（调用系统 git）
#[tauri::command]
pub async fn git_changes_remote_remove(root: String, name: String) -> Result<GitRemotes, GitError> {
    run_blocking(move || remote_remove_sync(&root, &name)).await
}

/// 拉取远端更新（remote 为空时取默认远端；网络操作放宽超时到 10 分钟）
#[tauri::command]
pub async fn git_changes_remote_fetch(root: String, remote: String) -> Result<GitBranches, GitError> {
    run_blocking_with_timeout(600, move || remote_fetch_sync(&root, &remote)).await
}

/// 从远程分支检出为同名本地跟踪分支（调用系统 git）
#[tauri::command]
pub async fn git_changes_branch_checkout_remote(
    root: String,
    remote_branch: String,
) -> Result<GitStatus, GitError> {
    run_blocking(move || branch_checkout_remote_sync(&root, &remote_branch)).await
}

/// 删除远程分支（调用系统 git；网络操作放宽超时到 10 分钟）
#[tauri::command]
pub async fn git_changes_remote_branch_delete(
    root: String,
    remote_branch: String,
) -> Result<GitBranches, GitError> {
    run_blocking_with_timeout(600, move || remote_branch_delete_sync(&root, &remote_branch)).await
}

/// 把当前分支上游切换到指定远端的同名分支（调用系统 git，单上游替换）
#[tauri::command]
pub async fn git_changes_remote_switch_upstream(
    root: String,
    remote: String,
) -> Result<GitRemotes, GitError> {
    run_blocking(move || remote_switch_upstream_sync(&root, &remote)).await
}

/// 探测本机是否安装了 git（推送依赖系统 git）；只探测不执行任何操作
#[tauri::command]
pub async fn git_changes_git_available(path: String) -> bool {
    let _ = path;
    find_git().is_some()
}

#[tauri::command]
pub async fn git_changes_init(path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || init_sync(&path)).await
}

#[tauri::command]
pub async fn git_changes_branches(path: String) -> Result<GitBranches, GitError> {
    run_blocking(move || branches_sync(&path)).await
}

#[tauri::command]
pub async fn git_changes_branch_create(path: String, name: String) -> Result<GitStatus, GitError> {
    run_blocking(move || create_branch_sync(&path, &name)).await
}

#[tauri::command]
pub async fn git_changes_branch_delete(path: String, name: String) -> Result<GitStatus, GitError> {
    run_blocking(move || delete_branch_sync(&path, &name)).await
}

#[tauri::command]
pub async fn git_changes_branch_switch(path: String, name: String) -> Result<GitStatus, GitError> {
    run_blocking(move || switch_branch_sync(&path, &name)).await
}

#[tauri::command]
pub async fn git_changes_branch_merge(path: String, name: String) -> Result<GitMergeResult, GitError> {
    run_blocking(move || merge_branch_sync(&path, &name)).await
}

#[tauri::command]
pub async fn git_changes_log(
    root: String,
    limit: usize,
    before: Option<String>,
) -> Result<Vec<GitCommitEntry>, GitError> {
    run_blocking(move || commit_log_sync(&root, limit, before)).await
}

#[tauri::command]
pub async fn git_changes_diff(root: String, path: String, kind: String) -> Result<String, GitError> {
    run_blocking(move || diff_sync(&root, &path, &kind)).await
}

#[tauri::command]
pub async fn git_changes_stage(root: String, path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || stage_sync(&root, &path)).await
}

#[tauri::command]
pub async fn git_changes_unstage(root: String, path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || unstage_sync(&root, &path)).await
}

#[tauri::command]
pub async fn git_changes_stage_all(root: String) -> Result<GitStatus, GitError> {
    run_blocking(move || stage_all_sync(&root)).await
}

#[tauri::command]
pub async fn git_changes_unstage_all(root: String) -> Result<GitStatus, GitError> {
    run_blocking(move || unstage_all_sync(&root)).await
}

#[tauri::command]
pub async fn git_changes_restore(root: String, path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || restore_sync(&root, &path)).await
}

#[tauri::command]
pub async fn git_changes_delete(root: String, path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || delete_sync(&root, &path)).await
}

#[tauri::command]
pub async fn git_changes_ignore(root: String, path: String) -> Result<GitStatus, GitError> {
    run_blocking(move || ignore_sync(&root, &path)).await
}

// ---------- .git / 工作区监听 ----------

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
            if s.eq_ignore_ascii_case("node_modules") && acc.is_dir() {
                return true;
            }
        }
    }
    false
}

/// 相对路径（正斜杠、小写比较）是否属于 .git 内部
fn is_git_internal_rel(rel_norm: &str) -> bool {
    let rel = rel_norm.to_ascii_lowercase();
    rel == ".git" || rel.starts_with(".git/")
}

/// 重新读取索引中的已跟踪路径集合（小写、正斜杠），用于 .git 变化后刷新监听过滤
fn load_tracked_set(root: &Path) -> std::collections::HashSet<String> {
    gix::discover(root)
        .ok()
        .and_then(|repo| repo.index_or_empty().ok())
        .map(|index| {
            index
                .entries()
                .iter()
                .map(|e| e.path(&index).to_str_lossy().to_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub async fn git_changes_watch_start(
    app: AppHandle,
    state: State<'_, GitWatcherState>,
    root: String,
) -> Result<(), GitError> {
    git_changes_watch_start_impl(&app, &state, &root).await
}

/// 开始监听仓库目录（远程 RPC 与 Tauri 命令共用）
pub async fn git_changes_watch_start_impl(
    app: &AppHandle,
    state: &GitWatcherState,
    root: &str,
) -> Result<(), GitError> {
    // 在阻塞线程中解析仓库：工作区根、git 目录、已跟踪路径集合、排除规则栈
    let (repo_root, git_dir, tracked, excludes, objects) = match run_blocking({
        let root = root.to_string();
        move || -> Result<
            (
                PathBuf,
                PathBuf,
                std::collections::HashSet<String>,
                gix::worktree::Stack,
                gix::OdbHandle,
            ),
            GitError,
        > {
            let repo = open_repo(&root)?;
            let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
            let index = repo
                .index_or_empty()
                .map_err(|e| git_err(format!("读取索引失败: {e}")))?;
            let tracked: std::collections::HashSet<String> = index
                .entries()
                .iter()
                .map(|e| e.path(&index).to_str_lossy().to_lowercase())
                .collect();
            // 排除栈脱离仓库，仅保留跨线程安全的对象句柄
            let stack = repo
                .excludes(
                    &index,
                    None,
                    gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
                )
                .map_err(|e| git_err(format!("初始化 gitignore 规则失败: {e}")))?
                .detach();
            let objects = repo.objects.clone();
            Ok((
                workdir.to_path_buf(),
                repo.git_dir().to_path_buf(),
                tracked,
                stack,
                objects,
            ))
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
    let excludes = Arc::new(Mutex::new((excludes, objects)));
    let tracked = Arc::new(Mutex::new(tracked));
    let tracked_for_task = tracked.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            for p in ev.paths {
                if path_under_excluded_dot_dir(&root_for_filter, &p) {
                    continue;
                }
                // 应用 .gitignore 规则：忽略“被忽略且未被跟踪”的路径；.git 内部事件始终放行
                if let Ok(rel) = p.strip_prefix(&root_for_filter) {
                    let rel_norm = rel.to_string_lossy().replace('\\', "/");
                    let is_git_internal = is_git_internal_rel(&rel_norm);
                    let is_tracked = tracked
                        .lock()
                        .map(|g| g.contains(&rel_norm.to_lowercase()))
                        .unwrap_or(false);
                    if !is_git_internal && !is_tracked {
                        if let Ok(mut guard) = excludes.lock() {
                            let (stack, objects) = &mut *guard;
                            let mode = if p.is_dir() {
                                Some(gix::index::entry::Mode::DIR)
                            } else {
                                None
                            };
                            let excluded = stack
                                .at_path(rel_norm.as_str(), mode, objects)
                                .map(|platform| platform.is_excluded())
                                .unwrap_or(false);
                            if excluded {
                                continue;
                            }
                        }
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

    // 防抖任务：300ms 静默后向前端 emit 变更事件
    let handle = app.clone();
    let root_for_task = repo_root.clone();
    let git_dir_for_task = git_dir.clone();
    tauri::async_runtime::spawn(async move {
        let mut last_event: Option<tokio::time::Instant> = None;
        let mut ticker = tokio::time::interval(Duration::from_millis(150));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(p) => {
                        // .git 内部事件（索引/引用变化）后刷新已跟踪集合，
                        // 避免新暂存/新提交的文件被 gitignore 过滤掉
                        let git_event = if let Ok(rel) = p.strip_prefix(&root_for_task) {
                            is_git_internal_rel(&rel.to_string_lossy().replace('\\', "/"))
                        } else {
                            p.starts_with(&git_dir_for_task)
                        };
                        if git_event {
                            let new_set =
                                tokio::task::block_in_place(|| load_tracked_set(&root_for_task));
                            if let Ok(mut g) = tracked_for_task.lock() {
                                *g = new_set;
                            }
                        }
                        last_event = Some(tokio::time::Instant::now());
                    }
                    None => break,
                },
                _ = ticker.tick() => {
                    if let Some(t) = last_event {
                        if t.elapsed() >= Duration::from_millis(300) {
                            let payload = serde_json::json!({ "root": clean_path(&root_for_task) });
                            crate::codex::remote::emit_event(
                                &handle,
                                "git-changes/changed",
                                payload,
                            );
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
    git_changes_watch_stop_impl(&state).await
}

pub async fn git_changes_watch_stop_impl(state: &GitWatcherState) -> Result<(), GitError> {
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
        // -b main：与 gix::init 的默认分支保持一致
        assert_eq!(git(dir, &["init", "-b", "main"]).0, 0);
        assert_eq!(git(dir, &["config", "user.name", "t"]).0, 0);
        assert_eq!(git(dir, &["config", "user.email", "t@t"]).0, 0);
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
        let st = status_sync(root.to_str().unwrap()).unwrap();
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

        let st = status_sync(root.to_str().unwrap()).unwrap();
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
        let st = status_sync(root.to_str().unwrap()).unwrap();
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
        let st = status_sync(root.to_str().unwrap()).unwrap();
        let paths: Vec<&str> = st.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["a.txt"]);
    }

    #[test]
    fn init_creates_repo_without_commit() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let st = init_sync(root.to_str().unwrap()).unwrap();
        assert!(root.join(".git").is_dir());
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].path, "a.txt");
        assert_eq!(st.files[0].status, FileStatus::Untracked);
        assert!(!st.files[0].staged);
        assert!(st.files[0].worktree);
        // 仅初始化，不应产生任何提交（HEAD 未出生）
        let repo = open_repo(root.to_str().unwrap()).unwrap();
        assert!(repo.head().unwrap().is_unborn());
    }

    #[test]
    fn init_twice_errors() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let _ = init_sync(root.to_str().unwrap()).unwrap();
        let err = init_sync(root.to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, GitErrorCode::RepoError);
        assert!(err.message.contains("已经是 Git 仓库"));
    }

    #[test]
    fn status_not_a_repo() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("x.txt"), "x").unwrap();
        let err = status_sync(dir.path().to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, GitErrorCode::NotARepo);
    }

    #[test]
    fn diff_untracked_returns_new_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("c.txt"), "hello\n").unwrap();
        let st = init_sync(root.to_str().unwrap()).unwrap();
        let diff = diff_sync(&st.repo_root, "c.txt", "untracked").unwrap();
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
        let st = status_sync(root.to_str().unwrap()).unwrap();
        let diff = diff_sync(&st.repo_root, "a.txt", "modified").unwrap();
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
        let st = status_sync(root.to_str().unwrap()).unwrap();
        let diff = diff_sync(&st.repo_root, "a.txt", "deleted").unwrap();
        assert!(diff.contains("-one"));
        assert!(diff.contains("-two"));
        assert!(!diff.contains("+one"));
    }

    #[test]
    fn diff_binary_errors() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("bin.dat"), b"\x00\x01\x02").unwrap();
        let st = init_sync(root.to_str().unwrap()).unwrap();
        let err = diff_sync(&st.repo_root, "bin.dat", "untracked").unwrap_err();
        assert!(err.message.contains("二进制"));
    }

    #[test]
    fn status_does_not_write_index() {
        // gix 状态只读：初始化后（无 index 文件）执行状态不应创建 .git/index
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let _ = init_sync(root.to_str().unwrap()).unwrap();
        assert!(!root.join(".git").join("index").exists());
        let st = status_sync(root.to_str().unwrap()).unwrap();
        assert!(!root.join(".git").join("index").exists());
        assert_eq!(st.files.len(), 1);
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
        let bs = branches_sync(root.to_str().unwrap()).unwrap();
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
        let st = create_branch_sync(root.to_str().unwrap(), "dev").unwrap();
        assert_eq!(st.branch, "main");
        let bs = branches_sync(root.to_str().unwrap()).unwrap();
        assert!(bs.branches.contains(&"dev".to_string()));
        // 重复创建同名分支报错
        assert!(create_branch_sync(root.to_str().unwrap(), "dev").is_err());
    }

    #[test]
    fn branch_create_requires_commit() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let _ = init_sync(root.to_str().unwrap()).unwrap();
        let err = create_branch_sync(root.to_str().unwrap(), "dev").unwrap_err();
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
        let st = delete_branch_sync(root.to_str().unwrap(), "feature").unwrap();
        assert_eq!(st.branch, "main");
        let bs = branches_sync(root.to_str().unwrap()).unwrap();
        assert!(!bs.branches.contains(&"feature".to_string()));
        // 禁止删除当前分支
        let err = delete_branch_sync(root.to_str().unwrap(), "main").unwrap_err();
        assert!(err.message.contains("当前所在分支"));
        // 删除不存在分支报错
        assert!(delete_branch_sync(root.to_str().unwrap(), "nope").is_err());
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
        let st = switch_branch_sync(root.to_str().unwrap(), "other").unwrap();
        assert_eq!(st.branch, "other");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "two");
        assert!(st.files.is_empty());

        // 切回 main
        let st = switch_branch_sync(root.to_str().unwrap(), "main").unwrap();
        assert_eq!(st.branch, "main");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one");
    }

    #[test]
    fn branch_switch_refuses_when_dirty() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        init_committed_repo(root);
        assert_eq!(git(root, &["branch", "other"]).0, 0);
        std::fs::write(root.join("a.txt"), "dirty").unwrap();
        let err = switch_branch_sync(root.to_str().unwrap(), "other").unwrap_err();
        assert!(err.message.contains("已跟踪的改动"));
        let repo = open_repo(root.to_str().unwrap()).unwrap();
        assert_eq!(repo.head_name().unwrap().unwrap().shorten(), "main");
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
        let err = switch_branch_sync(root.to_str().unwrap(), "other").unwrap_err();
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
        let st = switch_branch_sync(root.to_str().unwrap(), "other").unwrap();
        assert_eq!(st.branch, "other");
        assert_eq!(std::fs::read_to_string(root.join("docs/a.txt")).unwrap(), "two");

        // 切回 main
        let st = switch_branch_sync(root.to_str().unwrap(), "main").unwrap();
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
        let err = switch_branch_sync(root.to_str().unwrap(), "other").unwrap_err();
        assert!(err.message.contains("未跟踪文件将被覆盖"));
        assert!(err.message.contains("newdir/x.txt"));
    }

    #[test]
    fn stage_then_unstage_untracked_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("c.txt"), "hello\n").unwrap();
        let _ = init_sync(root.to_str().unwrap()).unwrap();

        // 暂存 → added 且 staged=true
        let st = stage_sync(root.to_str().unwrap(), "c.txt").unwrap();
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].path, "c.txt");
        assert_eq!(st.files[0].status, FileStatus::Added);
        assert!(st.files[0].staged);
        assert!(!st.files[0].worktree);

        // 取消暂存 → 回到 untracked 且 staged=false，工作区内容保留
        let st = unstage_sync(root.to_str().unwrap(), "c.txt").unwrap();
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].path, "c.txt");
        assert_eq!(st.files[0].status, FileStatus::Untracked);
        assert!(!st.files[0].staged);
        assert!(st.files[0].worktree);
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

        let st = stage_sync(root.to_str().unwrap(), "a.txt").unwrap();
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].status, FileStatus::Modified);
        assert!(st.files[0].staged);
        assert!(!st.files[0].worktree);

        let st = unstage_sync(root.to_str().unwrap(), "a.txt").unwrap();
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

        let st = stage_sync(root.to_str().unwrap(), "a.txt").unwrap();
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

        let st = stage_all_sync(root.to_str().unwrap()).unwrap();
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

        let st = stage_all_sync(root.to_str().unwrap()).unwrap();
        assert!(st.files.iter().all(|f| f.staged));

        let st = unstage_all_sync(root.to_str().unwrap()).unwrap();
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

        assert!(stage_all_sync(root.to_str().unwrap()).unwrap().files.is_empty());
        assert!(unstage_all_sync(root.to_str().unwrap())
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

        let st = restore_sync(root.to_str().unwrap(), "a.txt").unwrap();
        assert!(st.files.is_empty());
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\n");
    }

    #[test]
    fn restore_untracked_file_deletes_it() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("c.txt"), "hello\n").unwrap();
        let _ = init_sync(root.to_str().unwrap()).unwrap();

        let st = restore_sync(root.to_str().unwrap(), "c.txt").unwrap();
        assert!(st.files.is_empty());
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

        let st = delete_sync(root.to_str().unwrap(), "a.txt").unwrap();
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
        let _ = init_sync(root.to_str().unwrap()).unwrap();

        let st = ignore_sync(root.to_str().unwrap(), "c.txt").unwrap();
        // c.txt 已被忽略，仅 .gitignore 自身作为新文件出现
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].path, ".gitignore");
        assert!(!st.files[0].staged);
        assert!(st.files[0].worktree);
        let ig = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(ig, "/c.txt\n");

        // 重复忽略幂等：条目不重复
        let _ = ignore_sync(root.to_str().unwrap(), "c.txt").unwrap();
        let ig = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(ig, "/c.txt\n");
    }

    #[test]
    fn ignore_appends_to_existing_gitignore_without_trailing_newline() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("c.txt"), "hello\n").unwrap();
        std::fs::write(root.join("d.txt"), "world\n").unwrap();
        std::fs::write(root.join(".gitignore"), "*.log").unwrap();
        let _ = init_sync(root.to_str().unwrap()).unwrap();

        let st = ignore_sync(root.to_str().unwrap(), "c.txt").unwrap();
        // c.txt 被忽略；d.txt 与 .gitignore 仍为未跟踪
        let mut paths: Vec<&str> = st.files.iter().map(|f| f.path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, vec![".gitignore", "d.txt"]);
        let ig = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(ig, "*.log\n/c.txt\n");
    }

    #[test]
    fn file_ops_reject_unsafe_paths() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let _ = init_sync(root.to_str().unwrap()).unwrap();

        assert!(stage_sync(root.to_str().unwrap(), "../outside").is_err());
        assert!(stage_sync(root.to_str().unwrap(), r"C:\outside").is_err());
        assert!(stage_sync(root.to_str().unwrap(), "node_modules/x.js").is_err());
        assert!(stage_sync(root.to_str().unwrap(), ".git/config").is_err());
        assert!(restore_sync(root.to_str().unwrap(), "a/../b").is_err());
        assert!(delete_sync(root.to_str().unwrap(), "/abs").is_err());
        assert!(ignore_sync(root.to_str().unwrap(), "").is_err());
    }

    #[test]
    fn diff_sync_rejects_unsafe_paths() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let _ = init_sync(root.to_str().unwrap()).unwrap();

        assert!(diff_sync(root.to_str().unwrap(), "../outside.txt", "modified").is_err());
        assert!(diff_sync(root.to_str().unwrap(), r"C:\outside.txt", "modified").is_err());
        assert!(diff_sync(root.to_str().unwrap(), "node_modules/x.js", "modified").is_err());
        assert!(diff_sync(root.to_str().unwrap(), "a/../b", "modified").is_err());
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

        let err = commit_sync(root.to_str().unwrap(), "msg").unwrap_err();
        assert!(err.message.contains("冲突"));
    }

    #[test]
    fn stage_rejects_oversized_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let _ = init_sync(root.to_str().unwrap()).unwrap();

        std::fs::write(root.join("big.bin"), vec![0u8; MAX_FILE_BYTES as usize + 1]).unwrap();
        let err = stage_sync(root.to_str().unwrap(), "big.bin").unwrap_err();
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

        let st = stage_sync(root.to_str().unwrap(), "newdir").unwrap();
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
        let repo = open_repo(root.to_str().unwrap()).unwrap();
        let index = repo.index_or_empty().unwrap();
        assert!(index.entry_by_path(BStr::new("newdir/x.log")).is_none());
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

        let st = stage_sync(root.to_str().unwrap(), "src").unwrap();
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

        let st = stage_sync(root.to_str().unwrap(), "src").unwrap();
        assert_eq!(st.files.len(), 2);
        assert!(st.files.iter().all(|f| f.staged));
        assert!(st.files.iter().all(|f| !f.worktree));

        let st = unstage_sync(root.to_str().unwrap(), "src").unwrap();
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

        let st = restore_sync(root.to_str().unwrap(), "src").unwrap();
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
        let _ = init_sync(root.to_str().unwrap()).unwrap();

        let st = ignore_sync(root.to_str().unwrap(), "newdir").unwrap();
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].path, ".gitignore");
        assert!(!st.files[0].staged);
        assert!(st.files[0].worktree);
        let ig = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(ig, "/newdir/\n");

        // 重复忽略幂等
        let _ = ignore_sync(root.to_str().unwrap(), "newdir").unwrap();
        let ig = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(ig, "/newdir/\n");
    }

    #[test]
    fn delete_directory_rejected() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("newdir")).unwrap();
        std::fs::write(root.join("newdir/a.txt"), "x\n").unwrap();
        let _ = init_sync(root.to_str().unwrap()).unwrap();

        let err = delete_sync(root.to_str().unwrap(), "newdir").unwrap_err();
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

        let st = commit_sync(root.to_str().unwrap(), "feat: update a").unwrap();
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
        let _ = init_sync(root.to_str().unwrap()).unwrap();
        assert_eq!(git(root, &["config", "user.name", "t"]).0, 0);
        assert_eq!(git(root, &["config", "user.email", "t@t"]).0, 0);
        assert_eq!(git(root, &["add", "a.txt"]).0, 0);

        let st = commit_sync(root.to_str().unwrap(), "first").unwrap();
        assert!(st.files.is_empty());
        assert_eq!(git(root, &["log", "-1", "--format=%s"]).1.trim(), "first");
    }

    #[test]
    fn commit_empty_message_errors() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let _ = init_sync(root.to_str().unwrap()).unwrap();
        let err = commit_sync(root.to_str().unwrap(), "   ").unwrap_err();
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
        let err = commit_sync(root.to_str().unwrap(), "msg").unwrap_err();
        assert!(err.message.contains("没有已暂存的更改"));
    }

    #[test]
    fn commit_without_user_config_errors() {
        if !git_available() {
            eprintln!("skip: 未安装 git");
            return;
        }
        // gix 会按 git 规则回退到全局/系统配置与身份环境变量；存在可解析身份时
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
        let err = commit_sync(root.to_str().unwrap(), "msg").unwrap_err();
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

        let res = pull_sync(work2.path().to_str().unwrap()).unwrap();
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
        let err = pull_sync(work2.path().to_str().unwrap()).unwrap_err();
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
        let err = pull_sync(work2.path().to_str().unwrap()).unwrap_err();
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
        let err = pull_sync(work2.path().to_str().unwrap()).unwrap_err();
        assert!(err.message.contains("会被拉取覆盖"));
        assert_eq!(
            std::fs::read_to_string(work2.path().join("a.txt")).unwrap(),
            "staged\n"
        );
        assert_eq!(git(work2.path(), &["rev-parse", "HEAD"]).1, before_head);
        let st = status_sync(work2.path().to_str().unwrap()).unwrap();
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

        let res = pull_sync(work2.path().to_str().unwrap()).unwrap();
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

        let res = pull_sync(work2.path().to_str().unwrap()).unwrap();
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
        let res = pull_sync(work2.path().to_str().unwrap()).unwrap();
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

        let err = pull_sync_with_git(
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

        let err = pull_sync(work.path().to_str().unwrap()).unwrap_err();
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

        let res = push_sync(work2.path().to_str().unwrap()).unwrap();
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
        assert_eq!(push_sync(work2.path().to_str().unwrap()).unwrap().kind, "pushed");

        let res = push_sync(work2.path().to_str().unwrap()).unwrap();
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

        let err = push_sync(work2.path().to_str().unwrap()).unwrap_err();
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

        let err = push_sync(work2.path().to_str().unwrap()).unwrap_err();
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

        let err = push_sync(work.path().to_str().unwrap()).unwrap_err();
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

        let err = push_sync_with_git(
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
        assert!(find_git().is_some());
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

        let res = remotes_sync(work2.path().to_str().unwrap()).unwrap();
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

        let res = remotes_sync(work.path().to_str().unwrap()).unwrap();
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
        let git_bin = find_git().unwrap();
        let url = bare.path().to_string_lossy();

        let res = remote_add_sync_with_git(
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
        let git_bin = find_git().unwrap();

        let err = remote_add_sync_with_git(
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
        let git_bin = find_git().unwrap();
        let root = work.path().to_str().unwrap();

        let err = remote_add_sync_with_git(&git_bin, root, "bad name", "https://x").unwrap_err();
        assert!(err.message.contains("远端名"), "actual: {}", err.message);
        let err = remote_add_sync_with_git(&git_bin, root, "up", "  ").unwrap_err();
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
        let git_bin = find_git().unwrap();
        let root = work.path().to_str().unwrap();
        let url = bare.path().to_string_lossy();
        let url2 = format!("{}/mirror", url);
        assert_eq!(
            remote_add_sync_with_git(&git_bin, root, "origin", &url)
                .unwrap()
                .remotes
                .len(),
            1
        );

        let res = remote_set_url_sync_with_git(&git_bin, root, "origin", &url2).unwrap();
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
        let git_bin = find_git().unwrap();

        let err = remote_set_url_sync_with_git(
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
        let git_bin = find_git().unwrap();
        let root = work.path().to_str().unwrap();
        let url = bare.path().to_string_lossy();
        assert_eq!(
            remote_add_sync_with_git(&git_bin, root, "origin", &url)
                .unwrap()
                .remotes
                .len(),
            1
        );

        let res = remote_remove_sync_with_git(&git_bin, root, "origin").unwrap();
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
        let git_bin = find_git().unwrap();

        let err = remote_remove_sync_with_git(
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
        let git_bin = find_git().unwrap();
        let root = work2.path().to_str().unwrap();
        assert_eq!(
            remotes_sync(root).unwrap().current.as_deref(),
            Some("origin")
        );

        let res = remote_remove_sync_with_git(&git_bin, root, "origin").unwrap();
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

        let err = remote_add_sync_with_git(missing, work.path().to_str().unwrap(), "up", "https://x")
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

        let bs = branches_sync(work2.path().to_str().unwrap()).unwrap();
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
        let git_bin = find_git().unwrap();
        let root = work2.path().to_str().unwrap();
        assert!(!branches_sync(root).unwrap().remote_branches.contains(&"origin/dev".to_string()));

        push_remote_branch(work1.path(), bare.path(), "dev");
        let res = remote_fetch_sync_with_git(&git_bin, root, "origin").unwrap();
        assert!(res.remote_branches.contains(&"origin/dev".to_string()));
        // remote 为空：取默认远端（origin）
        let res2 = remote_fetch_sync_with_git(&git_bin, root, "").unwrap();
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
        let err = remote_fetch_sync_with_git(
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
        let git_bin = find_git().unwrap();

        let st = branch_checkout_remote_sync_with_git(
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
        let bs = branches_sync(work2.path().to_str().unwrap()).unwrap();
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
        let git_bin = find_git().unwrap();

        let err = branch_checkout_remote_sync_with_git(
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
        let git_bin = find_git().unwrap();

        let err = branch_checkout_remote_sync_with_git(
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
        let err = branch_checkout_remote_sync_with_git(
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
        let git_bin = find_git().unwrap();

        let res = remote_branch_delete_sync_with_git(
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
        let git_bin = find_git().unwrap();

        let err = remote_branch_delete_sync_with_git(
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
        let err = remote_branch_delete_sync_with_git(
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
        let st = status_sync(work.path().to_str().unwrap()).unwrap();
        assert!(!st.has_remote);

        let bare = TempDir::new().unwrap();
        let work1 = TempDir::new().unwrap();
        let work2 = TempDir::new().unwrap();
        setup_pull_repo(bare.path(), work1.path(), work2.path());
        let st = status_sync(work2.path().to_str().unwrap()).unwrap();
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
        let git_bin = find_git().unwrap();
        let root = work2.path().to_str().unwrap();
        assert_eq!(
            git(work2.path(), &["remote", "add", "upstream", bare2.path().to_str().unwrap()]).0,
            0
        );
        assert_eq!(git(work2.path(), &["fetch", "upstream"]).0, 0);

        let res = remote_switch_upstream_sync_with_git(&git_bin, root, "upstream").unwrap();
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
        let git_bin = find_git().unwrap();

        let err = remote_switch_upstream_sync_with_git(
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
        let git_bin = find_git().unwrap();

        let err = remote_switch_upstream_sync_with_git(
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
        let git_bin = find_git().unwrap();

        let err = remote_switch_upstream_sync_with_git(
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

        let err = remote_switch_upstream_sync_with_git(
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

        let res = merge_branch_sync(root.to_str().unwrap(), "feature").unwrap();
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

        let res = merge_branch_sync(root.to_str().unwrap(), "feature").unwrap();
        assert_eq!(res.kind, "merged");
        assert!(res.status.files.is_empty());
        // 双方内容都在
        assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "feat\n");
        assert_eq!(std::fs::read_to_string(root.join("c.txt")).unwrap(), "main\n");
        // HEAD 为合并提交
        let head_subject = git(root, &["log", "-1", "--format=%s"]).1;
        assert!(head_subject.contains("Merge branch 'feature'"));
        // 合并后再次合并 → up to date
        let res = merge_branch_sync(root.to_str().unwrap(), "feature").unwrap();
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
        let err = merge_branch_sync(root.to_str().unwrap(), "side").unwrap_err();
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
        let err = merge_branch_sync(root.to_str().unwrap(), "main").unwrap_err();
        assert!(err.message.contains("自身"));
        // 不存在的分支
        let err = merge_branch_sync(root.to_str().unwrap(), "nope").unwrap_err();
        assert!(err.message.contains("不存在"));
        // 游离 HEAD 无法合并
        assert_eq!(git(root, &["checkout", "--detach", "main"]).0, 0);
        let err = merge_branch_sync(root.to_str().unwrap(), "feature").unwrap_err();
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

        let err = merge_branch_sync(root.to_str().unwrap(), "feature").unwrap_err();
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

        let log = commit_log_sync(root.to_str().unwrap(), 10, None).unwrap();
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
        let log = commit_log_sync(root.to_str().unwrap(), 3, None).unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(log[0].subject, "c4");
        assert_eq!(log[2].subject, "c2");
    }

    #[test]
    fn commit_log_empty_when_no_commits() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x\n").unwrap();
        let _ = init_sync(root.to_str().unwrap()).unwrap();
        let log = commit_log_sync(root.to_str().unwrap(), 10, None).unwrap();
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
        let first = commit_log_sync(root.to_str().unwrap(), 3, None).unwrap();
        assert_eq!(first.len(), 3);
        assert_eq!(first[0].subject, "c7");
        assert_eq!(first[2].subject, "c5");
        let second =
            commit_log_sync(root.to_str().unwrap(), 3, Some(first[2].hash.clone())).unwrap();
        assert_eq!(second.len(), 3);
        assert_eq!(second[0].subject, "c4");
        assert_eq!(second[2].subject, "c2");
        let third =
            commit_log_sync(root.to_str().unwrap(), 3, Some(second[2].hash.clone())).unwrap();
        assert_eq!(third.len(), 3);
        assert_eq!(third[0].subject, "c1");
        assert_eq!(third[2].subject, "init");
        // 游标为最后一条时返回空
        let tail = commit_log_sync(root.to_str().unwrap(), 3, Some(third[2].hash.clone())).unwrap();
        assert!(tail.is_empty());
        // 无效游标返回空，避免前端重复追加
        let bogus = commit_log_sync(root.to_str().unwrap(), 3, Some("0".repeat(40))).unwrap();
        assert!(bogus.is_empty());
    }
}
