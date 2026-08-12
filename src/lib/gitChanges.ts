/** git 命令错误码（与 Rust `GitError.code` 对应） */
export type GitErrorCode = "git_not_found" | "not_a_repo" | "repo_error";

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

/** 映射到现有 diff 窗口的 kind 取值 */
export function gitDiffKind(status: GitFileStatus): string {
  if (status === "added" || status === "untracked") return "add";
  if (status === "deleted") return "delete";
  return "modify";
}
