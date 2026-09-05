import { ref, type Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { askConfirm, setToast, toastError } from "./useCodex";
import { markGitStatusFresh, setGitOpInFlight } from "./useGitChanges";
import type { ActionMenuItem } from "./useActionMenu";
import type { GitFile, GitStatus } from "../lib/gitChanges";
import type { GitDirNode } from "../lib/gitTree";
import {
  ICON_DELETE,
  ICON_IGNORE,
  ICON_OPEN,
  ICON_RESTORE,
  ICON_STAGE,
  ICON_UNSTAGE,
} from "../lib/icons";

/** 变更分区：更改（工作区侧） / 暂存更改（HEAD→索引侧） */
export type GitSection = "changes" | "staged";

/**
 * Git 变更文件操作与右键菜单：
 * 单路径操作（暂存/取消暂存/忽略/撤消/删除）、全量操作（全部暂存/取消暂存），
 * 以及文件/目录行的右键菜单构建；成功后回写 gitStatus。
 */
export function useGitFileActions(options: {
  gitStatus: Ref<GitStatus | null>;
  openCtx: (e: MouseEvent, items: ActionMenuItem[]) => void;
  openDiff: (file: GitFile) => void;
}) {
  const gitActionBusy = ref(false);

  async function runGitOp(cmd: string, relPath: string) {
    if (gitActionBusy.value) return;
    const root = options.gitStatus.value?.repoWorkspace;
    if (!root) return;
    gitActionBusy.value = true;
    try {
      setGitOpInFlight(true);
      const st = await invoke<GitStatus>(cmd, { workspace: root, path: relPath });
      options.gitStatus.value = st;
      markGitStatusFresh();
    } catch (e) {
      setToast(toastError(e));
    } finally {
      setGitOpInFlight(false);
      gitActionBusy.value = false;
    }
  }

  /** 全部操作：暂存全部工作区变更 / 取消暂存全部已暂存变更 */
  async function runGitAllOp(cmd: string) {
    if (gitActionBusy.value) return;
    const root = options.gitStatus.value?.repoWorkspace;
    if (!root) return;
    gitActionBusy.value = true;
    try {
      setGitOpInFlight(true);
      const st = await invoke<GitStatus>(cmd, { workspace: root });
      options.gitStatus.value = st;
      markGitStatusFresh();
    } catch (e) {
      setToast(toastError(e));
    } finally {
      setGitOpInFlight(false);
      gitActionBusy.value = false;
    }
  }

  function stageFile(file: GitFile) {
    void runGitOp("git_changes_stage", file.path);
  }

  function unstageFile(file: GitFile) {
    void runGitOp("git_changes_unstage", file.path);
  }

  function ignoreFile(file: GitFile) {
    void runGitOp("git_changes_ignore", file.path);
  }

  async function restoreFile(file: GitFile) {
    if (gitActionBusy.value) return;
    const ok = await askConfirm({
      title: "撤消更改",
      message: `将丢弃「${file.path}」的所有本地更改（含已暂存内容），确定撤消吗？`,
      confirmLabel: "撤消更改",
    });
    if (!ok) return;
    void runGitOp("git_changes_restore", file.path);
  }

  async function deleteFile(file: GitFile) {
    if (gitActionBusy.value) return;
    const ok = await askConfirm({
      title: "删除文件",
      message: `确定删除「${file.path}」吗？工作区文件将被移除并记录为暂存删除，此操作不可恢复。`,
      confirmLabel: "删除",
    });
    if (!ok) return;
    void runGitOp("git_changes_delete", file.path);
  }

  function stageDir(node: GitDirNode) {
    void runGitOp("git_changes_stage", node.relPath);
  }

  function unstageDir(node: GitDirNode) {
    void runGitOp("git_changes_unstage", node.relPath);
  }

  function ignoreDir(node: GitDirNode) {
    void runGitOp("git_changes_ignore", node.relPath);
  }

  async function restoreDir(node: GitDirNode) {
    if (gitActionBusy.value) return;
    const ok = await askConfirm({
      title: "撤消更改",
      message: `将丢弃「${node.relPath}」目录下的所有本地更改（含已暂存内容），确定撤消吗？`,
      confirmLabel: "撤消更改",
    });
    if (!ok) return;
    void runGitOp("git_changes_restore", node.relPath);
  }

  function stageAll() {
    void runGitAllOp("git_changes_stage_all");
  }

  function unstageAll() {
    void runGitAllOp("git_changes_unstage_all");
  }

  /** 分区文件列表：更改区=工作区侧，暂存更改区=已暂存侧 */
  function sectionFiles(section: GitSection): GitFile[] {
    const st = options.gitStatus.value;
    if (!st) return [];
    return section === "changes"
      ? st.files.filter((f) => f.worktree)
      : st.files.filter((f) => f.staged);
  }

  /** 分区级撤消更改：一次确认后逐文件丢弃（与文件级同命令、同双侧语义） */
  async function restoreSection(section: GitSection) {
    if (gitActionBusy.value) return;
    const files = sectionFiles(section);
    if (!files.length) return;
    const label = section === "changes" ? "更改" : "暂存更改";
    const scope =
      section === "changes"
        ? "所有本地更改（含已暂存内容）"
        : "所有更改（含未暂存部分）";
    const ok = await askConfirm({
      title: "撤消更改",
      message: `将丢弃「${label}」分区 ${files.length} 个文件的${scope}，确定撤消吗？`,
      confirmLabel: "撤消更改",
    });
    if (!ok) return;
    for (const file of files) {
      await runGitOp("git_changes_restore", file.path);
    }
  }

  /** 分区标题右键菜单：全部暂存/取消暂存 + 分区级撤消更改（空分区不弹） */
  function openSectionCtx(section: GitSection, e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (!sectionFiles(section).length) return;
    const items: ActionMenuItem[] = [];
    if (section === "changes") {
      items.push({
        label: "暂存",
        icon: ICON_STAGE,
        action: () => stageAll(),
      });
    } else {
      items.push({
        label: "取消暂存",
        icon: ICON_UNSTAGE,
        action: () => unstageAll(),
      });
    }
    items.push({
      label: "撤消更改",
      icon: ICON_RESTORE,
      danger: true,
      action: () => void restoreSection(section),
    });
    options.openCtx(e, items);
  }

  function openFileCtx(section: GitSection, file: GitFile, e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    const items: ActionMenuItem[] = [
      {
        label: "打开",
        icon: ICON_OPEN,
        action: () => options.openDiff(file),
      },
    ];
    if (file.status === "modified" || file.status === "deleted") {
      if (section === "changes") {
        items.push({
          label: "暂存",
          icon: ICON_STAGE,
          action: () => stageFile(file),
        });
      } else {
        items.push({
          label: "取消暂存",
          icon: ICON_UNSTAGE,
          action: () => unstageFile(file),
        });
      }
      items.push({
        label: "撤消更改",
        icon: ICON_RESTORE,
        danger: true,
        action: () => void restoreFile(file),
      });
    } else if (file.status === "untracked") {
      items.push({
        label: "暂存",
        icon: ICON_STAGE,
        action: () => stageFile(file),
      });
      items.push({
        label: "忽略此本地项",
        icon: ICON_IGNORE,
        action: () => ignoreFile(file),
      });
      items.push({
        label: "删除文件",
        icon: ICON_DELETE,
        danger: true,
        action: () => void deleteFile(file),
      });
    } else if (file.status === "added") {
      items.push({
        label: "取消暂存",
        icon: ICON_UNSTAGE,
        action: () => unstageFile(file),
      });
      items.push({
        label: "删除文件",
        icon: ICON_DELETE,
        danger: true,
        action: () => void deleteFile(file),
      });
    }
    // conflicted / renamed 仅保留“打开”
    options.openCtx(e, items);
  }

  function openDirCtx(section: GitSection, node: GitDirNode, e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    const items: ActionMenuItem[] = [];
    if (section === "changes") {
      // 目录出现在更改区即含工作区侧更改
      items.push({
        label: "暂存",
        icon: ICON_STAGE,
        action: () => stageDir(node),
      });
      if (node.hasUntracked) {
        items.push({
          label: "忽略此本地项",
          icon: ICON_IGNORE,
          action: () => ignoreDir(node),
        });
      }
    } else {
      // 目录出现在暂存更改区即含已暂存更改
      items.push({
        label: "取消暂存",
        icon: ICON_UNSTAGE,
        action: () => unstageDir(node),
      });
    }
    items.push({
      label: "撤消更改",
      icon: ICON_RESTORE,
      danger: true,
      action: () => void restoreDir(node),
    });
    options.openCtx(e, items);
  }

  return {
    gitActionBusy,
    openFileCtx,
    openDirCtx,
    stageFile,
    unstageFile,
    ignoreFile,
    restoreFile,
    deleteFile,
    stageDir,
    unstageDir,
    ignoreDir,
    restoreDir,
    stageAll,
    unstageAll,
    restoreSection,
    openSectionCtx,
  };
}
