<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { askConfirm, store, toastError } from "../composables/useCodex";
import {
  gitErrorMsg,
  gitInitBusy,
  gitState,
  gitStatus,
  initGitRepo,
  refreshGitChanges,
  setGitChangesActive,
} from "../composables/useGitChanges";
import { sessionRoot } from "../composables/useSessionFs";
import {
  gitDiffKind,
  gitStatusIcon,
  gitStatusLabel,
  type GitFile,
  type GitStatus,
} from "../lib/gitChanges";

const props = defineProps<{ active: boolean }>();

watch(
  () => props.active,
  (v) => setGitChangesActive(v),
  { immediate: true },
);

const confirmInit = ref(false);

const branchLabel = computed(() => gitStatus.value?.branch ?? "");
const repoRoot = computed(() => gitStatus.value?.repoRoot ?? "");
const fileCount = computed(() => gitStatus.value?.files.length ?? 0);
const branchMenuOpen = ref(false);
const branches = ref<string[]>([]);
const newBranchName = ref("");
const branchBusy = ref(false);

const ICON_DIFF =
  "M11.5 9a2.5 2.5 0 0 0 0 5 2.5 2.5 0 0 0 0-5zM20 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm-3.21 14.21l-2.91-2.91c-.69.44-1.51.7-2.39.7C9.01 16 7 13.99 7 11.5S9.01 7 11.5 7 16 9.01 16 11.5c0 .88-.26 1.69-.7 2.39l2.91 2.9-1.42 1.42z";
const ICON_STAGE =
  "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm5 11h-4v4h-2v-4H7v-2h4V7h2v4h4v2z";
const ICON_UNSTAGE =
  "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm5 11H7v-2h10v2z";
const ICON_RESTORE =
  "M13 3c-4.97 0-9 4.03-9 9H1l3.89 3.89.07.14L9 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42C8.27 19.99 10.51 21 13 21c4.97 0 9-4.03 9-9s-4.03-9-9-9zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z";
const ICON_IGNORE =
  "M12 7c2.76 0 5 2.24 5 5 0 .65-.13 1.26-.36 1.83l2.92 2.92c1.51-1.26 2.7-2.89 3.43-4.75-1.73-4.39-6-7.5-11-7.5-1.4 0-2.74.25-3.98.7l2.16 2.16C10.74 7.13 11.35 7 12 7zM2 4.27l2.28 2.28.46.46C3.08 8.3 1.78 10.02 1 12c1.73 4.39 6 7.5 11 7.5 1.55 0 3.03-.3 4.38-.84l.42.42L19.73 22 21 20.73 3.27 3 2 4.27zM7.53 9.8l1.55 1.55c-.05.21-.08.43-.08.65 0 1.66 1.34 3 3 3 .22 0 .44-.03.65-.08l1.55 1.55c-.67.33-1.41.53-2.2.53-2.76 0-5-2.24-5-5 0-.79.2-1.53.53-2.2zm4.31-.78l3.15 3.15.02-.16c0-1.66-1.34-3-3-3l-.17.01z";
const ICON_DELETE =
  "M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z";
const ICON_FOLDER_CLOSED =
  "M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z";
const ICON_FOLDER_OPEN =
  "M20 6h-8l-2-2H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V8h16v10z";
const ICON_ARROW_RIGHT = "M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z";
const ICON_ARROW_DOWN =
  "M7.41 8.59L12 13.17l4.59-4.58L18 10l-6 6-6-6 1.41-1.41z";

interface CtxItem {
  label: string;
  icon: string;
  danger?: boolean;
  action: () => void;
}

/** 分区：更改（工作区侧） / 暂存更改（HEAD→索引侧） */
type GitSection = "changes" | "staged";

const ctxMenu = ref<{ x: number; y: number; items: CtxItem[] } | null>(null);
const gitActionBusy = ref(false);

interface GitFileNode {
  kind: "file";
  name: string;
  relPath: string;
  depth: number;
  file: GitFile;
}

interface GitDirNode {
  kind: "dir";
  name: string;
  relPath: string;
  depth: number;
  collapsed: boolean;
  childCount: number;
  hasStaged: boolean;
  hasUnstaged: boolean;
  hasUntracked: boolean;
  children: GitTreeNode[];
}

type GitTreeNode = GitFileNode | GitDirNode;

interface DirAcc {
  dirs: Map<string, DirAcc>;
  files: GitFile[];
}

/** 手动折叠的目录集合；未记录 = 默认展开，折叠状态跨刷新保留 */
const collapsedDirs = reactive(new Set<string>());

function buildGitTree(files: GitFile[]): GitTreeNode[] {
  const root: DirAcc = { dirs: new Map(), files: [] };
  for (const f of files) {
    const parts = f.path.split("/");
    let acc = root;
    for (let i = 0; i < parts.length - 1; i++) {
      const d = parts[i];
      let next = acc.dirs.get(d);
      if (!next) {
        next = { dirs: new Map(), files: [] };
        acc.dirs.set(d, next);
      }
      acc = next;
    }
    acc.files.push(f);
  }
  return dirNodes(root, "", 0);
}

function dirNodes(acc: DirAcc, relPath: string, depth: number): GitTreeNode[] {
  const nodes: GitTreeNode[] = [];
  for (const name of [...acc.dirs.keys()].sort((a, b) => a.localeCompare(b))) {
    const childRel = relPath ? `${relPath}/${name}` : name;
    const children = dirNodes(acc.dirs.get(name)!, childRel, depth + 1);
    const childCount = children.reduce(
      (n, c) => n + (c.kind === "dir" ? c.childCount : 1),
      0,
    );
    const hasStaged = children.some((c) =>
      c.kind === "dir" ? c.hasStaged : c.file.staged,
    );
    const hasUnstaged = children.some((c) =>
      c.kind === "dir" ? c.hasUnstaged : !c.file.staged,
    );
    const hasUntracked = children.some((c) =>
      c.kind === "dir" ? c.hasUntracked : c.file.status === "untracked",
    );
    nodes.push({
      kind: "dir",
      name,
      relPath: childRel,
      depth,
      collapsed: collapsedDirs.has(childRel),
      childCount,
      hasStaged,
      hasUnstaged,
      hasUntracked,
      children,
    });
  }
  const files = acc.files
    .slice()
    .sort((a, b) => a.path.localeCompare(b.path));
  for (const f of files) {
    nodes.push({
      kind: "file",
      name: f.path.split("/").pop() ?? f.path,
      relPath: f.path,
      depth,
      file: f,
    });
  }
  return nodes;
}

function flattenRows(nodes: GitTreeNode[]): GitTreeNode[] {
  const rows: GitTreeNode[] = [];
  for (const n of nodes) {
    rows.push(n);
    if (n.kind === "dir" && !n.collapsed) {
      rows.push(...flattenRows(n.children));
    }
  }
  return rows;
}

const worktreeFiles = computed(() =>
  gitStatus.value ? gitStatus.value.files.filter((f) => f.worktree) : [],
);
const stagedFiles = computed(() =>
  gitStatus.value ? gitStatus.value.files.filter((f) => f.staged) : [],
);
const worktreeRows = computed(() => flattenRows(buildGitTree(worktreeFiles.value)));
const stagedRows = computed(() => flattenRows(buildGitTree(stagedFiles.value)));

function startInit() {
  confirmInit.value = true;
}

async function doInit() {
  confirmInit.value = false;
  await initGitRepo();
}

async function openBranchMenu() {
  const root = repoRoot.value;
  if (!root || branchBusy.value) return;
  branchBusy.value = true;
  try {
    const res = await invoke<{ current: string; branches: string[] }>(
      "git_changes_branches",
      { path: root },
    );
    branches.value = res.branches;
    branchMenuOpen.value = true;
  } catch (e) {
    store.toast = toastError(e);
  } finally {
    branchBusy.value = false;
  }
}

async function toggleBranchMenu() {
  if (branchMenuOpen.value) {
    branchMenuOpen.value = false;
  } else {
    await openBranchMenu();
  }
}

async function switchBranch(name: string) {
  if (branchBusy.value || name === branchLabel.value) return;
  branchBusy.value = true;
  try {
    const st = await invoke<GitStatus>("git_changes_branch_switch", {
      path: repoRoot.value,
      name,
    });
    gitStatus.value = st;
    branchMenuOpen.value = false;
  } catch (e) {
    store.toast = toastError(e);
  } finally {
    branchBusy.value = false;
  }
}

async function createBranch() {
  const name = newBranchName.value.trim();
  if (!name || branchBusy.value) return;
  branchBusy.value = true;
  try {
    const st = await invoke<GitStatus>("git_changes_branch_create", {
      path: repoRoot.value,
      name,
    });
    gitStatus.value = st;
    newBranchName.value = "";
    // 重新拉取分支列表，保持弹层打开
    const res = await invoke<{ current: string; branches: string[] }>(
      "git_changes_branches",
      { path: repoRoot.value },
    );
    branches.value = res.branches;
  } catch (e) {
    store.toast = toastError(e);
  } finally {
    branchBusy.value = false;
  }
}

async function deleteBranch(name: string) {
  if (branchBusy.value || name === branchLabel.value) return;
  branchBusy.value = true;
  try {
    const st = await invoke<GitStatus>("git_changes_branch_delete", {
      path: repoRoot.value,
      name,
    });
    gitStatus.value = st;
    branches.value = branches.value.filter((b) => b !== name);
  } catch (e) {
    store.toast = toastError(e);
  } finally {
    branchBusy.value = false;
  }
}

function onWindowClick(e: MouseEvent) {
  // 用 Element 而非 HTMLElement：点击 svg/path 等 SVG 目标也应正确判断
  if (!(e.target instanceof Element)) {
    branchMenuOpen.value = false;
    ctxMenu.value = null;
    return;
  }
  if (
    e.target.closest(".git-branch-menu") ||
    e.target.closest(".git-branch-btn") ||
    e.target.closest(".ctx-menu")
  ) {
    return;
  }
  branchMenuOpen.value = false;
  ctxMenu.value = null;
}

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  branchMenuOpen.value = false;
  ctxMenu.value = null;
}

function onWindowScroll(e: Event) {
  // 仅面板自身滚动时关闭；聊天区等外部滚动不影响分支弹层
  if (!(e.target instanceof Element)) return;
  if (!e.target.closest(".git-view")) return;
  branchMenuOpen.value = false;
  ctxMenu.value = null;
}

onMounted(() => {
  window.addEventListener("click", onWindowClick);
  window.addEventListener("keydown", onKeydown);
  window.addEventListener("scroll", onWindowScroll, true);
});
onBeforeUnmount(() => {
  window.removeEventListener("click", onWindowClick);
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("scroll", onWindowScroll, true);
});

async function openDiff(file: GitFile) {
  const root = gitStatus.value?.repoRoot;
  if (!root) return;
  try {
    const diff = await invoke<string>("git_changes_diff", {
      root,
      path: file.path,
      kind: file.status,
    });
    if (!diff) {
      store.toast = "该文件无内容变化（可能仅为重命名）";
      return;
    }
    await invoke("open_diff_window", {
      params: {
        path: file.path,
        kind: gitDiffKind(file.status),
        diff,
        workspace_root: root,
      },
    });
  } catch (e) {
    store.toast = toastError(e);
  }
}

function openFileCtx(section: GitSection, file: GitFile, e: MouseEvent) {
  e.preventDefault();
  e.stopPropagation();
  const items: CtxItem[] = [
    {
      label: "查看更改",
      icon: ICON_DIFF,
      action: () => void openDiff(file),
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
  // conflicted / renamed 仅保留查看更改
  const x = Math.min(e.clientX, window.innerWidth - 190);
  const y = Math.min(e.clientY, window.innerHeight - items.length * 30 - 12);
  ctxMenu.value = { x, y, items };
}

function toggleDirRow(node: GitDirNode) {
  if (collapsedDirs.has(node.relPath)) collapsedDirs.delete(node.relPath);
  else collapsedDirs.add(node.relPath);
}

function openDirCtx(section: GitSection, node: GitDirNode, e: MouseEvent) {
  e.preventDefault();
  e.stopPropagation();
  const items: CtxItem[] = [];
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
  const x = Math.min(e.clientX, window.innerWidth - 190);
  const y = Math.min(e.clientY, window.innerHeight - items.length * 30 - 12);
  ctxMenu.value = { x, y, items };
}

async function runGitOp(cmd: string, relPath: string) {
  if (gitActionBusy.value) return;
  const root = gitStatus.value?.repoRoot;
  if (!root) return;
  gitActionBusy.value = true;
  try {
    const st = await invoke<GitStatus>(cmd, { root, path: relPath });
    gitStatus.value = st;
  } catch (e) {
    store.toast = toastError(e);
  } finally {
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
</script>

<template>
  <div class="git-view">
    <div v-if="gitState === 'loading'" class="git-note">正在检查 Git 状态…</div>

    <div v-else-if="gitState === 'not_repo'" class="git-empty">
      <p class="git-empty-title">当前目录不是 Git 仓库</p>
      <p class="git-empty-desc">
        点击下方按钮可将当前工作目录初始化为 Git 仓库（仅初始化，不会自动提交）。
      </p>
      <button
        class="btn git-init-btn"
        :disabled="gitInitBusy"
        @click="startInit"
      >
        {{ gitInitBusy ? "初始化中…" : "添加到 Git" }}
      </button>
    </div>

    <div v-else-if="gitState === 'error'" class="git-empty">
      <p class="git-empty-title">无法获取 Git 状态</p>
      <p class="git-empty-desc">{{ gitErrorMsg }}</p>
      <button class="btn git-init-btn" @click="refreshGitChanges()">重试</button>
    </div>

    <template v-else-if="gitState === 'ok' && gitStatus">
      <div class="git-head">
        <button
          class="git-branch git-branch-btn"
          v-tooltip="repoRoot"
          :disabled="branchBusy"
          @click="toggleBranchMenu()"
        >
          <svg class="git-branch-icon" viewBox="0 0 16 16" aria-hidden="true">
            <path
              d="M14 5.5C14 4.121 12.879 3 11.5 3C10.121 3 9 4.121 9 5.5C9 6.682 9.826 7.669 10.93 7.928C10.744 8.546 10.177 9 9.5 9H6.5C5.935 9 5.419 9.195 5 9.512V4.949C6.14 4.717 7 3.707 7 2.5C7 1.121 5.879 0 4.5 0C3.121 0 2 1.121 2 2.5C2 3.708 2.86 4.717 4 4.949V11.05C2.86 11.282 2 12.292 2 13.499C2 14.878 3.121 15.999 4.5 15.999C5.879 15.999 7 14.878 7 13.499C7 12.317 6.174 11.33 5.07 11.071C5.256 10.453 5.823 9.999 6.5 9.999H9.5C10.723 9.999 11.74 9.115 11.954 7.953C13.116 7.738 14 6.723 14 5.5ZM3 2.5C3 1.673 3.673 1 4.5 1C5.327 1 6 1.673 6 2.5C6 3.327 5.327 4 4.5 4C3.673 4 3 3.327 3 2.5ZM6 13.5C6 14.327 5.327 15 4.5 15C3.673 15 3 14.327 3 13.5C3 12.673 3.673 12 4.5 12C5.327 12 6 12.673 6 13.5ZM11.5 7C10.673 7 10 6.327 10 5.5C10 4.673 10.673 4 11.5 4C12.327 4 13 4.673 13 5.5C13 6.327 12.327 7 11.5 7Z"
            />
          </svg>
          <span class="git-branch-name-text">{{ branchLabel }}</span>
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M7.41 8.59L12 13.17l4.59-4.58L18 10l-6 6-6-6 1.41-1.41z"
            />
          </svg>
        </button>
        <span class="git-count">{{ fileCount }} 个更改</span>
        <button
          class="git-refresh"
          aria-label="刷新"
          v-tooltip="'刷新'"
          @click="refreshGitChanges()"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M17.65 6.35A7.95 7.95 0 0 0 12 4a8 8 0 1 0 7.73 10h-2.08A6 6 0 1 1 12 6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z"
            />
          </svg>
        </button>
        <!-- 分支弹层作为 .git-head 子节点，absolute 定位相对头部，避免被面板 overflow 裁掉 -->
        <div v-if="branchMenuOpen" class="git-branch-menu">
          <div class="git-branch-menu-list">
            <div
              v-for="b in branches"
              :key="b"
              class="git-branch-menu-item"
              :class="{ current: b === branchLabel }"
              v-tooltip="b === branchLabel ? '当前分支' : `切换到 ${b}`"
              @click="switchBranch(b)"
            >
              <span class="git-branch-check">
                {{ b === branchLabel ? "✓" : "" }}
              </span>
              <span class="git-branch-name">{{ b }}</span>
              <button
                v-if="b !== branchLabel"
                class="git-branch-delete"
                v-tooltip="'删除分支'"
                :disabled="branchBusy"
                @click.stop="deleteBranch(b)"
              >
                ×
              </button>
            </div>
            <div v-if="!branches.length" class="git-branch-menu-empty">
              暂无分支
            </div>
          </div>
          <div class="git-branch-create">
            <input
              v-model="newBranchName"
              class="git-branch-input"
              type="text"
              placeholder="新建分支…"
              :disabled="branchBusy"
              @keydown.enter="createBranch()"
            />
            <button
              class="git-branch-create-btn"
              :disabled="branchBusy || !newBranchName.trim()"
              @click="createBranch()"
            >
              新建
            </button>
          </div>
        </div>
      </div>

      <div class="git-section">
        <div class="git-section-head">
          <span>更改</span>
        </div>
        <div v-if="worktreeRows.length" class="git-file-list">
          <div
            v-for="row in worktreeRows"
            :key="`changes:${row.kind}:${row.relPath}`"
            class="git-tree-row"
            :class="row.kind === 'dir' ? 'git-dir' : 'git-file'"
            :style="{ paddingLeft: 8 + row.depth * 14 + 'px' }"
            @click="
              row.kind === 'dir'
                ? toggleDirRow(row)
                : openDiff(row.file)
            "
            @contextmenu="
              row.kind === 'dir'
                ? openDirCtx('changes', row, $event)
                : openFileCtx('changes', row.file, $event)
            "
          >
            <template v-if="row.kind === 'dir'">
              <svg class="git-dir-arrow" viewBox="0 0 24 24" aria-hidden="true">
                <path :d="row.collapsed ? ICON_ARROW_RIGHT : ICON_ARROW_DOWN" />
              </svg>
              <svg class="git-dir-icon" viewBox="0 0 24 24" aria-hidden="true">
                <path :d="row.collapsed ? ICON_FOLDER_CLOSED : ICON_FOLDER_OPEN" />
              </svg>
              <span class="git-dir-name">{{ row.name }}</span>
              <span class="git-dir-count">{{ row.childCount }}</span>
            </template>
            <template v-else>
              <span
                class="git-status-icon"
                :class="`git-status-${row.file.status}`"
                v-tooltip="gitStatusLabel(row.file.status)"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path :d="gitStatusIcon(row.file.status)" />
                </svg>
              </span>
              <span class="git-path">{{ row.file.path }}</span>
            </template>
          </div>
        </div>
        <div v-else class="git-section-empty">无更改</div>
      </div>

      <div class="git-section">
        <div class="git-section-head">
          <span>暂存更改</span>
        </div>
        <div v-if="stagedRows.length" class="git-file-list">
          <div
            v-for="row in stagedRows"
            :key="`staged:${row.kind}:${row.relPath}`"
            class="git-tree-row"
            :class="row.kind === 'dir' ? 'git-dir' : 'git-file'"
            :style="{ paddingLeft: 8 + row.depth * 14 + 'px' }"
            @click="
              row.kind === 'dir'
                ? toggleDirRow(row)
                : openDiff(row.file)
            "
            @contextmenu="
              row.kind === 'dir'
                ? openDirCtx('staged', row, $event)
                : openFileCtx('staged', row.file, $event)
            "
          >
            <template v-if="row.kind === 'dir'">
              <svg class="git-dir-arrow" viewBox="0 0 24 24" aria-hidden="true">
                <path :d="row.collapsed ? ICON_ARROW_RIGHT : ICON_ARROW_DOWN" />
              </svg>
              <svg class="git-dir-icon" viewBox="0 0 24 24" aria-hidden="true">
                <path :d="row.collapsed ? ICON_FOLDER_CLOSED : ICON_FOLDER_OPEN" />
              </svg>
              <span class="git-dir-name">{{ row.name }}</span>
              <span class="git-dir-count">{{ row.childCount }}</span>
            </template>
            <template v-else>
              <span
                class="git-status-icon"
                :class="`git-status-${row.file.status}`"
                v-tooltip="gitStatusLabel(row.file.status)"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path :d="gitStatusIcon(row.file.status)" />
                </svg>
              </span>
              <span class="git-path">{{ row.file.path }}</span>
            </template>
          </div>
        </div>
        <div v-else class="git-section-empty">无暂存更改</div>
      </div>
    </template>

    <div
      v-if="ctxMenu"
      class="ctx-menu"
      :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }"
      @click.stop
    >
      <button
        v-for="it in ctxMenu.items"
        :key="it.label"
        class="ctx-menu-item"
        :class="{ danger: it.danger }"
        :disabled="gitActionBusy"
        @click="it.action(); ctxMenu = null"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path :d="it.icon" />
        </svg>
        <span>{{ it.label }}</span>
      </button>
    </div>

    <div v-if="confirmInit" class="modal-mask" @click.self="confirmInit = false">
      <div class="modal" tabindex="-1">
        <div class="modal-head">
          <span class="modal-title">添加到 Git</span>
          <button
            class="modal-close"
            aria-label="关闭"
            @click="confirmInit = false"
          >
            ×
          </button>
        </div>
        <div class="modal-body">
          确定要将当前目录初始化为 Git 仓库吗？<br />
          <span class="git-confirm-path">
            {{ sessionRoot || "当前工作目录" }}
          </span>
          <p class="git-confirm-desc">
            将执行 git init，仅初始化、不会自动提交；初始化后现有文件会以“未跟踪”状态显示。
          </p>
        </div>
        <div class="modal-foot">
          <button class="btn" @click="confirmInit = false">取消</button>
          <button class="btn git-init-ok" :disabled="gitInitBusy" @click="doInit">
            {{ gitInitBusy ? "初始化中…" : "确认" }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
