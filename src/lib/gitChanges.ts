/** git 命令错误码（与 Rust `GitError.code` 对应） */
export type GitErrorCode = "not_a_repo" | "repo_error";

export type GitFileStatus =
  | "added"
  | "modified"
  | "deleted"
  | "renamed"
  | "untracked"
  | "conflicted";

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
  repoRoot: string;
  /** 当前分支名；游离 HEAD 时为 "HEAD" */
  branch: string;
  files: GitFile[];
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

export function gitStatusLabel(status: GitFileStatus): string {
  switch (status) {
    case "added":
      return "新增";
    case "modified":
      return "修改";
    case "deleted":
      return "删除";
    case "renamed":
      return "重命名";
    case "untracked":
      return "未跟踪";
    case "conflicted":
      return "冲突";
  }
}

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

/** 映射到现有 diff 窗口的 kind 取值 */
export function gitDiffKind(status: GitFileStatus): string {
  if (status === "added" || status === "untracked") return "add";
  if (status === "deleted") return "delete";
  return "modify";
}
