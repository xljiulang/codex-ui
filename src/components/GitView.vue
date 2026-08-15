<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import GitBranchMenu from "./GitBranchMenu.vue";
import { askConfirm, setToast, toastError } from "../composables/useCodex";
import { useActionMenu, type CtxItem } from "../composables/useActionMenu";
import {
  gitErrorMsg,
  gitInitBusy,
  gitRevealTarget,
  gitState,
  gitStatus,
  initGitRepo,
  refreshGitChanges,
  setGitChangesActive,
} from "../composables/useGitChanges";
import { workspace } from "../composables/useCodex";
import { ensureEntryIcons, iconFor } from "../composables/useSessionFs";
import { openDiffTab } from "../composables/useEditorTabs";
import { joinFsPath, type FsEntry } from "../lib/sessionFs";
import {
  gitStatusLetter,
  normalizeDiffKind,
  type GitCommitEntry,
  type GitFile,
  type GitPullResult,
  type GitPushResult,
  type GitStatus,
} from "../lib/gitChanges";
import { formatDateTime } from "../lib/format";
import {
  ICON_ARROW_DOWN,
  ICON_ARROW_RIGHT,
  ICON_DELETE,
  ICON_FOLDER_CLOSED,
  ICON_FOLDER_OPEN,
  ICON_OPEN,
  ICON_PLUS,
} from "../lib/icons";

const props = defineProps<{ active: boolean }>();

/** 本机是否安装了 git（推送依赖系统 git，缺失时禁用按钮并提示） */
const gitAvailable = ref(true);
const branchLabel = computed(() => gitStatus.value?.branch ?? "");
const repoWorkspace = computed(() => gitStatus.value?.repoWorkspace ?? "");
/** 当前在 Git 面板中高亮的变更文件（相对仓库根路径） */
const selectedGitPath = ref("");

watch(
  () => props.active,
  (v) => {
    setGitChangesActive(v);
    if (v) void checkGitAvailable();
    // 面板切换进入时无条件重试：diff 标签激活早于 git 状态加载也能定位
    if (v) attemptReveal();
  },
  { immediate: true },
);

const confirmInit = ref(false);

const branchMenuOpen = ref(false);
/** 分支弹层忙碌态（由 GitBranchMenu v-model 同步，用于头部按钮禁用） */
const branchBusy = ref(false);
const pullBusy = ref(false);
const pushBusy = ref(false);
const commitMessage = ref("");
const commitBusy = ref(false);

/** 提交历史（最新在前），git_changes_log 返回 */
const commits = ref<GitCommitEntry[]>([]);
const logBusy = ref(false);
/** 是否还有更旧的提交可加载（上一批返回满 50 条即视为还有更多） */
const logHasMore = ref(false);
const LOG_LIMIT = 50;

/** 通用文件回退图标（与资源面板一致） */
const ICON_FILE =
  "M14 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z";

const ICON_STAGE =
  "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm5 11h-4v4h-2v-4H7v-2h4V7h2v4h4v2z";
const ICON_UNSTAGE =
  "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm5 11H7v-2h10v2z";
const ICON_RESTORE =
  "M13 3c-4.97 0-9 4.03-9 9H1l3.89 3.89.07.14L9 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42C8.27 19.99 10.51 21 13 21c4.97 0 9-4.03 9-9s-4.03-9-9-9zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z";
const ICON_IGNORE =
  "M12 7c2.76 0 5 2.24 5 5 0 .65-.13 1.26-.36 1.83l2.92 2.92c1.51-1.26 2.7-2.89 3.43-4.75-1.73-4.39-6-7.5-11-7.5-1.4 0-2.74.25-3.98.7l2.16 2.16C10.74 7.13 11.35 7 12 7zM2 4.27l2.28 2.28.46.46C3.08 8.3 1.78 10.02 1 12c1.73 4.39 6 7.5 11 7.5 1.55 0 3.03-.3 4.38-.84l.42.42L19.73 22 21 20.73 3.27 3 2 4.27zM7.53 9.8l1.55 1.55c-.05.21-.08.43-.08.65 0 1.66 1.34 3 3 3 .22 0 .44-.03.65-.08l1.55 1.55c-.67.33-1.41.53-2.2.53-2.76 0-5-2.24-5-5 0-.79.2-1.53.53-2.2zm4.31-.78l3.15 3.15.02-.16c0-1.66-1.34-3-3-3l-.17.01z";
const ICON_ARROW_UP =
  "M4 12l1.41 1.41L11 7.83V20h2V7.83l5.58 5.59L20 12l-8-8-8 8z";
const ICON_CHECK =
  "M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41L9 16.17z";
const ICON_MORE =
  "M6 10c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm12 0c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm-6 0c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z";

/** 分区：更改（工作区侧） / 暂存更改（HEAD→索引侧） */
type GitSection = "changes" | "staged";

const {
  ctxMenu,
  openCtx,
  onWindowClick: onMenuWindowClick,
  onWindowScroll: onMenuWindowScroll,
  onKeydown: onMenuKeydown,
} = useActionMenu({ width: 190, scrollScope: ".git-view" });
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

/** 可折叠分区 key：changes | staged | history；未记录 = 默认展开。
 * 历史记录默认折叠（内容较多），更改/暂存区默认展开。折叠状态跨刷新保留。 */
const collapsedSections = reactive(new Set<string>(["history"]));

function isSectionCollapsed(key: string): boolean {
  return collapsedSections.has(key);
}

function toggleSection(key: string) {
  if (collapsedSections.has(key)) collapsedSections.delete(key);
  else collapsedSections.add(key);
}

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
const stagedCount = computed(() => stagedFiles.value.length);
/** 更改总数（每个文件计一次，含已暂存 + 工作区 + 未跟踪） */
const changeCount = computed(() => gitStatus.value?.files.length ?? 0);

/** 归一化目标为相对仓库根路径；仓库不匹配返回 null */
function normGitRel(target: { workspace: string; path: string }): string | null {
  const root = repoWorkspace.value;
  if (!root) return null;
  if (target.workspace && target.workspace !== root) return null;
  const rootNorm = root.replace(/\\/g, "/").replace(/\/+$/, "");
  let p = target.path.replace(/\\/g, "/");
  if (p.startsWith(rootNorm + "/")) {
    p = p.slice(rootNorm.length + 1);
  }
  return p || null;
}

/** 在 Git 面板中定位并高亮目标变更文件：展开祖先目录、滚动到可见 */
function attemptReveal() {
  const target = gitRevealTarget.value;
  if (!target) return;
  const rel = normGitRel(target);
  if (rel === null) return;
  // 在完整文件列表中查找（折叠的祖先目录不影响定位），变更区优先、暂存区次之
  const match =
    worktreeFiles.value.find((f) => f.path === rel) ??
    stagedFiles.value.find((f) => f.path === rel);
  if (!match) return;
  const parts = rel.split("/");
  let acc = "";
  for (let i = 0; i < parts.length - 1; i++) {
    acc = acc ? `${acc}/${parts[i]}` : parts[i];
    collapsedDirs.delete(acc);
  }
  selectedGitPath.value = rel;
  void nextTick(() => {
    document
      .querySelector<HTMLElement>(`[data-git-path="${CSS.escape(rel)}"]`)
      ?.scrollIntoView({ block: "center" });
  });
}

// diff 标签激活请求 / git 状态晚加载：定位并高亮对应变更行
watch([gitRevealTarget, gitStatus], () => attemptReveal());

const canCommit = computed(
  () =>
    !commitBusy.value &&
    !!commitMessage.value.trim() &&
    stagedCount.value > 0,
);

/** 变更文件 → FsEntry（供系统文件图标管线复用；repoWorkspace 为空时回退相对路径） */
function gitFileEntry(file: GitFile): FsEntry {
  const root = repoWorkspace.value;
  return {
    name: file.path.split("/").pop() ?? file.path,
    path: root ? joinFsPath(root, file.path) : file.path,
    relPath: file.path,
    isDir: false,
    size: null,
    modifiedAtMs: 0,
    createdAtMs: 0,
    childCount: null,
  };
}

/** 文件类型图标：共享图标缓存取不到时返回 undefined，渲染层回退 SVG */
function gitFileIcon(file: GitFile): string | undefined {
  return iconFor(gitFileEntry(file)) ?? undefined;
}

// 变更列表可见行变化时懒加载缺失的文件图标（与资源面板同管线）
watch(
  [worktreeRows, stagedRows, repoWorkspace],
  () => {
    const root = repoWorkspace.value;
    if (!root) return;
    const files = [
      ...worktreeRows.value,
      ...stagedRows.value,
    ]
      .filter((r): r is GitFileNode => r.kind === "file")
      .map((r) => gitFileEntry(r.file));
    if (files.length) void ensureEntryIcons(files, root);
  },
);

const commitHint = computed(() =>
  stagedCount.value > 0
    ? `将提交 ${stagedCount.value} 个文件`
    : "先在上方暂存更改",
);

/** 提交已暂存更改；成功后清空消息并刷新状态 */
async function doCommit() {
  if (!canCommit.value) return;
  const root = gitStatus.value?.repoWorkspace;
  if (!root) return;
  commitBusy.value = true;
  try {
    const st = await invoke<GitStatus>("git_changes_commit", {
      workspace: root,
      message: commitMessage.value.trim(),
    });
    gitStatus.value = st;
    commitMessage.value = "";
    setToast("提交成功");
  } catch (e) {
    setToast(toastError(e));
  } finally {
    commitBusy.value = false;
  }
}

/** 拉取远端更新；游离 HEAD 或忙碌时禁用 */
async function doPull() {
  if (pullBusy.value || branchLabel.value === "HEAD" || !gitAvailable.value) return;
  const root = repoWorkspace.value;
  if (!root) return;
  pullBusy.value = true;
  try {
    const res = await invoke<GitPullResult>("git_changes_pull", { workspace: root });
    gitStatus.value = res.status;
    setToast(res.message);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    pullBusy.value = false;
  }
}

/** 探测本机是否安装了 git（推送可用性）；探测失败视为不可用 */
async function checkGitAvailable() {
  try {
    gitAvailable.value = await invoke<boolean>("git_changes_git_available", {
      path: repoWorkspace.value || workspace.value || "",
    });
  } catch {
    gitAvailable.value = false;
  }
}

/** 推送当前分支到上游；游离 HEAD、未装 git 或忙碌时禁用 */
async function doPush() {
  if (
    pushBusy.value ||
    pullBusy.value ||
    branchLabel.value === "HEAD" ||
    !gitAvailable.value
  ) {
    return;
  }
  const root = repoWorkspace.value;
  if (!root) return;
  pushBusy.value = true;
  try {
    const res = await invoke<GitPushResult>("git_changes_push", { workspace: root });
    gitStatus.value = res.status;
    setToast(res.message);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    pushBusy.value = false;
  }
}

function startInit() {
  confirmInit.value = true;
}

async function doInit() {
  confirmInit.value = false;
  await initGitRepo();
}

/** 分支弹层开关：打开后由 GitBranchMenu 挂载时自行加载列表与远端 */
async function toggleBranchMenu() {
  branchMenuOpen.value = !branchMenuOpen.value;
}

/** 分支/远端操作导致仓库状态变化：回写 gitStatus（联动提交历史刷新） */
function applyBranchStatus(st: GitStatus) {
  gitStatus.value = st;
}

/** 重载提交历史第一页（最新在前）；由 gitStatus 变化/手动刷新触发 */
async function loadCommitLog() {
  const root = repoWorkspace.value;
  if (!root || logBusy.value) return;
  logBusy.value = true;
  try {
    const res = await invoke<GitCommitEntry[]>("git_changes_log", {
      workspace: root,
      limit: LOG_LIMIT,
      before: null,
    });
    commits.value = Array.isArray(res) ? res : [];
    logHasMore.value = commits.value.length === LOG_LIMIT;
  } catch {
    commits.value = [];
    logHasMore.value = false;
  } finally {
    logBusy.value = false;
  }
}

/** 加载更多提交历史：以当前最后一条 hash 为游标续页并追加 */
async function loadMoreCommits() {
  const root = repoWorkspace.value;
  const last = commits.value[commits.value.length - 1];
  if (!root || logBusy.value || !last) return;
  logBusy.value = true;
  try {
    const res = await invoke<GitCommitEntry[]>("git_changes_log", {
      workspace: root,
      limit: LOG_LIMIT,
      before: last.hash,
    });
    if (!Array.isArray(res) || !res.length) {
      // 已到历史尽头（含游标失效/重写场景）：隐藏按钮，保留已加载列表
      logHasMore.value = false;
      return;
    }
    commits.value = commits.value.concat(res);
    logHasMore.value = res.length === LOG_LIMIT;
  } catch {
    // 加载更多失败保持现状，按钮保留以便重试
  } finally {
    logBusy.value = false;
  }
}

// gitStatus 每次刷新（含提交/合并/拉取/切分支）后同步刷新提交历史
watch(
  () => gitStatus.value,
  () => void loadCommitLog(),
);

function onWindowClick(e: MouseEvent) {
  // 右键菜单：任意外部 click 关闭（弹层内部点击由各自处理器负责）
  if (e.target instanceof Element && e.target.closest(".ctx-menu")) return;
  onMenuWindowClick();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  onMenuKeydown(e);
}

function onWindowScroll(e: Event) {
  // 右键菜单：仅面板自身滚动时关闭；聊天区等外部滚动不影响
  if (!(e.target instanceof Element)) return;
  if (!e.target.closest(".git-view")) return;
  onMenuWindowScroll(e);
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
  const root = gitStatus.value?.repoWorkspace;
  if (!root) return;
  try {
    const diff = await invoke<string>("git_changes_diff", {
      workspace: root,
      path: file.path,
      kind: normalizeDiffKind(file.status),
    });
    if (!diff) {
      setToast("该文件无内容变化（可能仅为重命名）");
      return;
    }
    await openDiffTab({
      path: file.path,
      kind: normalizeDiffKind(file.status),
      diff,
      workspace: root,
    });
  } catch (e) {
    setToast(toastError(e));
  }
}

function openFileCtx(section: GitSection, file: GitFile, e: MouseEvent) {
  e.preventDefault();
  e.stopPropagation();
  const items: CtxItem[] = [
    {
      label: "打开",
      icon: ICON_OPEN,
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
  // conflicted / renamed 仅保留“打开”
  openCtx(e, items);
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
  openCtx(e, items);
}

async function runGitOp(cmd: string, relPath: string) {
  if (gitActionBusy.value) return;
  const root = gitStatus.value?.repoWorkspace;
  if (!root) return;
  gitActionBusy.value = true;
  try {
    const st = await invoke<GitStatus>(cmd, { workspace: root, path: relPath });
    gitStatus.value = st;
  } catch (e) {
    setToast(toastError(e));
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

/** 全部操作：暂存全部工作区变更 / 取消暂存全部已暂存变更 */
async function runGitAllOp(cmd: string) {
  if (gitActionBusy.value) return;
  const root = gitStatus.value?.repoWorkspace;
  if (!root) return;
  gitActionBusy.value = true;
  try {
    const st = await invoke<GitStatus>(cmd, { workspace: root });
    gitStatus.value = st;
  } catch (e) {
    setToast(toastError(e));
  } finally {
    gitActionBusy.value = false;
  }
}

function stageAll() {
  void runGitAllOp("git_changes_stage_all");
}

function unstageAll() {
  void runGitAllOp("git_changes_unstage_all");
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
        class="btn primary git-init-btn"
        :disabled="gitInitBusy"
        @click="startInit"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path :d="ICON_PLUS" />
        </svg>
        <span>{{ gitInitBusy ? "初始化中…" : "添加到 Git" }}</span>
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
          v-tooltip="repoWorkspace"
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
        <div class="git-pull-push">
          <button
            class="git-icon-btn git-pull"
            :class="{ busy: pullBusy }"
            :disabled="
              !gitStatus?.hasRemote ||
              !gitAvailable ||
              pullBusy ||
              pushBusy ||
              branchLabel === 'HEAD'
            "
            :aria-label="pullBusy ? '拉取中…' : '拉取'"
            v-tooltip="
              !gitStatus?.hasRemote
                ? '未配置远端，无法拉取'
                : !gitAvailable
                ? '未检测到 git，无法拉取'
                : branchLabel === 'HEAD'
                  ? '游离 HEAD 无法拉取'
                  : pullBusy
                    ? '拉取中…'
                    : pushBusy
                      ? '推送中…'
                      : '拉取'
            "
            @click="doPull()"
          >
            <svg v-if="!pullBusy" viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z"
              />
            </svg>
            <span v-else class="git-pull-text">拉取中…</span>
          </button>
          <span class="git-pull-push-divider" aria-hidden="true"></span>
          <button
            class="git-icon-btn git-push"
            :class="{ busy: pushBusy }"
            :disabled="
              !gitStatus?.hasRemote ||
              !gitAvailable ||
              pushBusy ||
              pullBusy ||
              branchLabel === 'HEAD'
            "
            :aria-label="pushBusy ? '推送中…' : '推送'"
            v-tooltip="
              !gitStatus?.hasRemote
                ? '未配置远端，无法推送'
                : !gitAvailable
                ? '未检测到 git，无法推送'
                : branchLabel === 'HEAD'
                  ? '游离 HEAD 无法推送'
                  : pushBusy
                    ? '推送中…'
                    : pullBusy
                      ? '拉取中…'
                      : '推送'
            "
            @click="doPush()"
          >
            <svg v-if="!pushBusy" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M9 16h6v-6h4l-7-7-7 7h4v6zm-4 2h14v2H5v-2z" />
            </svg>
            <span v-else class="git-push-text">推送中…</span>
          </button>
        </div>
        <!-- 分支弹层作为 .git-head 子节点，absolute 定位相对头部，避免被面板 overflow 裁掉 -->
        <GitBranchMenu
          v-if="branchMenuOpen"
          :workspace="repoWorkspace"
          :branch-label="branchLabel"
          v-model:busy="branchBusy"
          @close="branchMenuOpen = false"
          @status="applyBranchStatus"
        />
      </div>

      <div class="git-section" :class="{ collapsed: isSectionCollapsed('changes') }">
        <div
          class="git-section-head"
          role="button"
          tabindex="0"
          :aria-expanded="!isSectionCollapsed('changes')"
          @click="toggleSection('changes')"
          @keydown.enter="toggleSection('changes')"
        >
          <svg class="git-section-arrow" viewBox="0 0 24 24" aria-hidden="true">
            <path
              :d="
                isSectionCollapsed('changes')
                  ? ICON_ARROW_RIGHT
                  : ICON_ARROW_DOWN
              "
            />
          </svg>
          <span>更改</span>
          <span class="git-section-actions">
            <span v-if="changeCount > 0" class="git-section-count">{{
              changeCount
            }}</span>
            <button
              class="git-icon-btn git-section-action git-section-stage"
              aria-label="全部暂存"
              v-tooltip="'全部暂存'"
              :disabled="gitActionBusy || !worktreeRows.length"
              @click.stop="stageAll()"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_ARROW_DOWN" />
              </svg>
            </button>
          </span>
        </div>
        <template v-if="!isSectionCollapsed('changes')">
          <div v-if="worktreeRows.length" class="git-file-list">
            <div
              v-for="row in worktreeRows"
              :key="`changes:${row.kind}:${row.relPath}`"
              class="git-tree-row"
              :class="{
                'git-dir': row.kind === 'dir',
                'git-file': row.kind === 'file',
                selected: row.kind === 'file' && row.relPath === selectedGitPath,
              }"
              :data-git-path="row.kind === 'file' ? row.relPath : undefined"
              :style="{ paddingLeft: 8 + row.depth * 14 + 'px' }"
              @click="row.kind === 'dir' ? toggleDirRow(row) : openDiff(row.file)"
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
              </template>
              <template v-else>
                <span
                  class="git-file-icon"
                  aria-hidden="true"
                >
                  <img
                    v-if="gitFileIcon(row.file)"
                    class="git-file-icon-img"
                    :src="gitFileIcon(row.file)"
                    alt=""
                    draggable="false"
                  />
                  <svg v-else viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_FILE" />
                  </svg>
                </span>
                <span
                  class="git-path"
                  :class="{ 'git-path-strike': row.file.status === 'deleted' }"
                >
                  {{ row.file.path }}
                </span>
                <span
                  class="git-status-icon"
                  :class="`git-status-${row.file.status}`"
                >
                  {{ gitStatusLetter(row.file.status) }}
                </span>
              </template>
            </div>
          </div>
          <div v-else class="git-section-empty">无更改</div>
        </template>
      </div>

      <div class="git-section" :class="{ collapsed: isSectionCollapsed('staged') }">
        <div
          class="git-section-head"
          role="button"
          tabindex="0"
          :aria-expanded="!isSectionCollapsed('staged')"
          @click="toggleSection('staged')"
          @keydown.enter="toggleSection('staged')"
        >
          <svg class="git-section-arrow" viewBox="0 0 24 24" aria-hidden="true">
            <path
              :d="
                isSectionCollapsed('staged')
                  ? ICON_ARROW_RIGHT
                  : ICON_ARROW_DOWN
              "
            />
          </svg>
          <span>暂存更改</span>
          <span class="git-section-actions">
            <span v-if="stagedCount > 0" class="git-section-count">{{
              stagedCount
            }}</span>
            <button
              class="git-icon-btn git-section-action git-section-unstage"
              aria-label="全部取消暂存"
              v-tooltip="'全部取消暂存'"
              :disabled="gitActionBusy || !stagedRows.length"
              @click.stop="unstageAll()"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_ARROW_UP" />
              </svg>
            </button>
          </span>
        </div>
        <template v-if="!isSectionCollapsed('staged')">
          <div class="git-commit-bar">
            <textarea
              v-model="commitMessage"
              class="git-commit-input"
              rows="2"
              placeholder="提交消息（Ctrl+Enter 提交）"
              :disabled="commitBusy"
              @keydown.ctrl.enter="doCommit()"
            ></textarea>
            <div class="git-commit-row">
              <button
                class="git-commit-btn"
                :disabled="!canCommit"
                @click="doCommit()"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path :d="ICON_CHECK" />
                </svg>
                <span>{{ commitBusy ? "提交中…" : "提交" }}</span>
              </button>
              <span class="git-commit-hint">{{ commitHint }}</span>
            </div>
          </div>
          <div v-if="stagedRows.length" class="git-file-list">
            <div
              v-for="row in stagedRows"
              :key="`staged:${row.kind}:${row.relPath}`"
              class="git-tree-row"
              :class="{
                'git-dir': row.kind === 'dir',
                'git-file': row.kind === 'file',
                selected: row.kind === 'file' && row.relPath === selectedGitPath,
              }"
              :data-git-path="row.kind === 'file' ? row.relPath : undefined"
              :style="{ paddingLeft: 8 + row.depth * 14 + 'px' }"
              @click="row.kind === 'dir' ? toggleDirRow(row) : openDiff(row.file)"
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
              </template>
              <template v-else>
                <span
                  class="git-file-icon"
                  aria-hidden="true"
                >
                  <img
                    v-if="gitFileIcon(row.file)"
                    class="git-file-icon-img"
                    :src="gitFileIcon(row.file)"
                    alt=""
                    draggable="false"
                  />
                  <svg v-else viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_FILE" />
                  </svg>
                </span>
                <span
                  class="git-path"
                  :class="{ 'git-path-strike': row.file.status === 'deleted' }"
                >
                  {{ row.file.path }}
                </span>
                <span
                  class="git-status-icon"
                  :class="`git-status-${row.file.status}`"
                >
                  {{ gitStatusLetter(row.file.status) }}
                </span>
              </template>
            </div>
          </div>
          <div v-else class="git-section-empty">无暂存更改</div>
        </template>
      </div>

      <div class="git-section" :class="{ collapsed: isSectionCollapsed('history') }">
        <div
          class="git-section-head"
          role="button"
          tabindex="0"
          :aria-expanded="!isSectionCollapsed('history')"
          @click="toggleSection('history')"
          @keydown.enter="toggleSection('history')"
        >
          <svg class="git-section-arrow" viewBox="0 0 24 24" aria-hidden="true">
            <path
              :d="
                isSectionCollapsed('history')
                  ? ICON_ARROW_RIGHT
                  : ICON_ARROW_DOWN
              "
            />
          </svg>
          <span>提交历史</span>
        </div>
        <template v-if="!isSectionCollapsed('history')">
          <div v-if="commits.length" class="git-log-list">
            <div v-for="c in commits" :key="c.hash" class="git-log-item">
              <span class="git-log-dot" aria-hidden="true"></span>
              <div class="git-log-main">
                <span class="git-log-subject">{{ c.subject }}</span>
                <span class="git-log-meta">
                  {{ c.author }} · {{ formatDateTime(c.timeSecs) }}
                </span>
              </div>
              <span class="git-log-hash" :title="c.hash">{{ c.shortHash }}</span>
            </div>
            <button
              v-if="logHasMore"
              class="git-icon-btn git-log-more"
              aria-label="加载更多"
              v-tooltip="'加载更多'"
              :disabled="logBusy"
              @click="loadMoreCommits()"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_MORE" />
              </svg>
            </button>
          </div>
          <div v-else-if="logBusy" class="git-section-empty">加载中…</div>
          <div v-else class="git-section-empty">暂无提交记录</div>
        </template>
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
            {{ workspace || "当前工作目录" }}
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
