/** git 命令错误码（与 Rust `GitError.code` 对应） */
export type GitErrorCode = "not_a_repo" | "repo_error";

export type GitFileStatus =
  "added" | "modified" | "deleted" | "renamed" | "untracked" | "conflicted";

/** 变更文件条目（Rust git_changes_status 返回，字段 camelCase） */
export interface GitFile {
  /** 相对仓库根的路径（正斜杠分隔） */
  path: string;
  status: GitFileStatus;
  /** 是否已有暂存区变化（HEAD → 索引） */
  staged: boolean;
  /** 是否含工作区侧变化（索引 → 工作区，含未跟踪文件） */
  worktree: boolean;
}

export interface GitStatus {
  /** 仓库根目录（Windows 反斜杠路径） */
  repoWorkspace: string;
  /** 当前分支名；游离 HEAD 时为 "HEAD" */
  branch: string;
  /** 仓库是否配置了任意远端（拉取/推送可用性的前置条件） */
  hasRemote: boolean;
  files: GitFile[];
}

/** 分支列表（Rust git_changes_branches 返回，字段 camelCase） */
export interface GitBranches {
  /** 当前分支名；游离 HEAD 或尚未出生时为 "HEAD" */
  current: string;
  /** 本地分支名（按名称排序） */
  branches: string[];
  /** 远端跟踪分支短名（如 origin/main，按名称排序） */
  remoteBranches: string[];
  /** 当前分支上游的「远端/分支」短名；无上游时为 null */
  currentUpstream: string | null;
}

/** Rust git_changes_pull 返回值（字段 camelCase） */
export interface GitPullResult {
  /** 拉取后的最新状态 */
  status: GitStatus;
  /** up_to_date | fast_forward | merged */
  kind: "up_to_date" | "fast_forward" | "merged";
  /** 给用户的中文结果提示 */
  message: string;
}

/** Rust git_changes_push 返回值（字段 camelCase） */
export interface GitPushResult {
  /** 推送后的最新状态 */
  status: GitStatus;
  /** pushed | up_to_date */
  kind: "pushed" | "up_to_date";
  /** 给用户的中文结果提示 */
  message: string;
}

/** Rust git_changes_branch_merge 返回值（与拉取结果同构） */
export type GitMergeResult = GitPullResult;

/** Rust git_changes_log 单条提交记录（字段 camelCase） */
export interface GitCommitEntry {
  /** 完整提交哈希 */
  hash: string;
  /** 7 位短哈希 */
  shortHash: string;
  /** 提交标题（首行） */
  subject: string;
  /** 作者名 */
  author: string;
  /** UNIX 秒时间戳（作者时区） */
  timeSecs: number;
}

/** Rust git_changes_commit_detail 单文件条目（字段 camelCase） */
export interface GitCommitFile {
  /** 相对仓库根的路径（正斜杠分隔；重命名取新路径） */
  path: string;
  status: GitFileStatus;
  /** 新增行数（二进制为 0） */
  insertions: number;
  /** 删除行数（二进制为 0） */
  deletions: number;
  /** 是否二进制文件（git numstat 显示 -） */
  binary: boolean;
}

/** Rust git_changes_commit_detail 返回值（字段 camelCase） */
export interface GitCommitDetail {
  /** 完整提交哈希 */
  hash: string;
  /** 7 位短哈希 */
  shortHash: string;
  /** 提交标题（首行） */
  subject: string;
  /** 完整提交消息（标题 + 正文，保留换行） */
  body: string;
  /** 作者名 */
  author: string;
  /** 作者邮箱 */
  authorEmail: string;
  /** 作者 UNIX 秒时间戳（作者时区） */
  authorTimeSecs: number;
  /** 提交者名 */
  committer: string;
  /** 提交者邮箱 */
  committerEmail: string;
  /** 提交者 UNIX 秒时间戳（提交者时区） */
  committerTimeSecs: number;
  /** 父提交完整哈希（合并提交多个） */
  parents: string[];
  /** 变更文件 */
  files: GitCommitFile[];
}

/** 远端条目（Rust git_changes_remotes 返回，字段 camelCase） */
export interface GitRemote {
  /** 远端名 */
  name: string;
  /** 拉取地址（remote.<name>.url；仅配置推送地址时可能为 null） */
  fetchUrl: string | null;
  /** 推送地址（未显式配置 pushUrl 时与拉取地址相同） */
  pushUrl: string | null;
}

/** 远端列表（Rust git_changes_remotes 返回） */
export interface GitRemotes {
  /** 当前分支跟踪/默认使用的远端名；无远端时为 null */
  current: string | null;
  /** 按名称排序的远端列表 */
  remotes: GitRemote[];
}

/** diff 预览类型（与 DiffPreviewParams.kind 一致） */
export type DiffPreviewKind = "add" | "delete" | "modify";

/** 状态文字图标（git 惯例：A/M/D/R/U/C） */
export function gitStatusLetter(status: GitFileStatus): string {
  switch (status) {
    case "added":
      return "A";
    case "modified":
      return "M";
    case "deleted":
      return "D";
    case "renamed":
      return "R";
    case "untracked":
      return "U";
    case "conflicted":
      return "C";
  }
}

/**
 * 任意变更 kind → diff 预览类型（唯一实现）：
 * 兼容 GitFileStatus（added/untracked→add、deleted→delete）与协议 kind
 * （字符串或 { type } 对象）；其余一律按修改处理。
 */
export function normalizeDiffKind(kind: unknown): DiffPreviewKind {
  const k =
    typeof kind === "string"
      ? kind
      : kind &&
          typeof kind === "object" &&
          "type" in (kind as Record<string, unknown>)
        ? String((kind as { type: unknown }).type)
        : "";
  if (k === "add" || k === "added" || k === "untracked") return "add";
  if (k === "delete" || k === "deleted") return "delete";
  return "modify";
}

/** diff 预览类型的中文标签（新增/删除/修改） */
export function diffKindLabel(kind: DiffPreviewKind): string {
  if (kind === "add") return "新增";
  if (kind === "delete") return "删除";
  return "修改";
}

/**
 * 默认折叠未变更行的行数阈值：内联 diff 会给「hunk 之外未变化的每一行」出行，
 * 大文件（几万行）即使只改一行也会渲染几万行 DOM，故超过该行数时默认进简要显示。
 */
export const DIFF_AUTO_BRIEF_ROWS = 2000;

/** diff 行数是否达到「默认简要显示（只渲染变更行）」的阈值 */
export function shouldBriefDiffRows(rowCount: number): boolean {
  return rowCount > DIFF_AUTO_BRIEF_ROWS;
}
