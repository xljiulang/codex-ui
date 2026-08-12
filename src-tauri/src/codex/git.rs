use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gix::bstr::{BStr, BString, ByteSlice};
use gix::progress::Discard;
use gix::status::tree_index::TrackRenames;
use gix::status::UntrackedFiles;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

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

/// 变更文件条目
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFile {
    /// 相对仓库根的路径（正斜杠分隔）
    pub path: String,
    /// added | modified | deleted | untracked
    pub status: String,
}

/// Git 状态（供前端展示）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    /// 仓库根目录（Windows 反斜杠路径）
    pub repo_root: String,
    /// 当前分支名；游离 HEAD 或尚未出生时为 "HEAD"
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
    status: &str,
) {
    let prio = |s: &str| match s {
        "deleted" => 4,
        "added" => 3,
        "modified" => 2,
        "untracked" => 1,
        _ => 0,
    };
    let norm = path.to_lowercase();
    if let Some(&i) = index_of.get(&norm) {
        if prio(status) > prio(&files[i].status) {
            files[i].status = status.to_string();
        }
    } else {
        index_of.insert(norm, files.len());
        files.push(GitFile {
            path,
            status: status.to_string(),
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
                        merge_file(&mut files, &mut index_of, path_str(&location), "added");
                    }
                    Change::Deletion { location, .. } => {
                        merge_file(&mut files, &mut index_of, path_str(&location), "deleted");
                    }
                    Change::Modification { location, .. } => {
                        merge_file(&mut files, &mut index_of, path_str(&location), "modified");
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
                            EntryStatus::Conflict { .. } => "modified",
                            EntryStatus::Change(Change::Removed) => "deleted",
                            EntryStatus::Change(_) => "modified",
                            EntryStatus::NeedsUpdate(_) => continue,
                            EntryStatus::IntentToAdd => "added",
                        };
                        merge_file(&mut files, &mut index_of, path_str(rela_path.as_ref()), status);
                    }
                    IwItem::DirectoryContents { entry, .. } => {
                        if matches!(entry.status, gix::dir::entry::Status::Untracked) {
                            merge_file(
                                &mut files,
                                &mut index_of,
                                path_str(entry.rela_path.as_ref()),
                                "untracked",
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
    files.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
    Ok(GitStatus {
        repo_root: clean_path(workdir),
        branch,
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
    branches.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    Ok(GitBranches { current, branches })
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
                // 逐级清理空的父目录
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

    status_sync(path)
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
        Ok(std::mem::take(&mut obj.data))
    } else {
        Ok(Vec::new())
    }
}

/// diff 基准统一为 HEAD（暂存 + 未暂存合并）；未跟踪文件为空 → 工作区内容。
/// 返回空串表示无内容变化。
fn diff_sync(root: &str, path: &str, _kind: &str) -> Result<String, GitError> {
    let repo = open_repo(root)?;
    let workdir = repo.workdir().ok_or_else(|| git_err("该仓库没有工作目录"))?;
    let old = head_blob(&repo, path)?;
    // 已删除或不可读的文件视为空内容
    let new = std::fs::read(workdir.join(path)).unwrap_or_default();
    if old == new {
        return Ok(String::new());
    }
    render_diff(&old, &new).ok_or_else(|| git_err("该文件是二进制文件，无法显示文本差异"))
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
pub async fn git_changes_diff(root: String, path: String, kind: String) -> Result<String, GitError> {
    run_blocking(move || diff_sync(&root, &path, &kind)).await
}

// ---------- .git / 工作区监听 ----------

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
            if s.eq_ignore_ascii_case("node_modules") && acc.is_dir() {
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
    // 在阻塞线程中解析仓库：工作区根、git 目录、已跟踪路径集合、排除规则栈
    let (repo_root, git_dir, tracked, excludes, objects) = match run_blocking({
        let root = root.clone();
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
    let tracked = Arc::new(tracked);
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            if let Some(p) = ev.paths.first() {
                if path_under_excluded_dot_dir(&root_for_filter, p) {
                    return;
                }
                // 应用 .gitignore 规则：忽略“被忽略且未被跟踪”的路径；.git 内部事件始终放行
                if let Ok(rel) = p.strip_prefix(&root_for_filter) {
                    let rel_norm = rel.to_string_lossy().replace('\\', "/");
                    let is_git_internal = rel_norm == ".git" || rel_norm.starts_with(".git/");
                    let is_tracked = tracked.contains(&rel_norm.to_lowercase());
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
                                return;
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
        assert_eq!(st.files[0].status, "modified");
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
        assert_eq!(st.files[0].status, "untracked");
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
}
