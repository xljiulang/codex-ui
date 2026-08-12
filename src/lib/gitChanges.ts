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

/** 状态图标（Material 24×24 路径），圆形符号风格 */
export function gitStatusIcon(status: GitFileStatus): string {
  switch (status) {
    case "added":
      // add_circle：加号圆
      return "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm5 11h-4v4h-2v-4H7v-2h4V7h2v4h4v2z";
    case "modified":
      // edit：铅笔
      return "M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04c.39-.39.39-1.02 0-1.41l-2.34-2.34a.9959.9959 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z";
    case "deleted":
      // remove_circle：减号圆
      return "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm5 11H7v-2h10v2z";
    case "untracked":
      // help：问号圆
      return "M11 18h2v-2h-2v2zm1-16C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8zm0-14c-2.21 0-4 1.79-4 4h2c0-1.1.9-2 2-2s2 .9 2 2c0 2-3 1.75-3 5h2c0-2.25 3-2.5 3-5 0-2.21-1.79-4-4-4z";
    case "renamed":
      // swap_horiz：交换箭头
      return "M6.99 11L3 15l3.99 4v-3H14v-2H6.99v-3zM21 9l-3.99-4v3H10v2h7.01v3L21 9z";
    case "conflicted":
      // error：感叹号圆
      return "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-2h2v2zm0-4h-2V7h2v6z";
  }
}

/** 映射到现有 diff 窗口的 kind 取值 */
export function gitDiffKind(status: GitFileStatus): string {
  if (status === "added" || status === "untracked") return "add";
  if (status === "deleted") return "delete";
  return "modify";
}
