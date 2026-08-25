import { nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { setToast, toastError, workspace } from "../useCodex";
import { activeTab } from "../useEditorTabs";
import { TabKind } from "../../lib/tabs";
import { relPathOf } from "../../lib/format";
import {
  isPathUnderRoot,
  joinFsPath,
  type FsEntry,
} from "../../lib/sessionFs";
import { normalizeFsPath, pathEquals } from "../../lib/path";
import {
  childrenByPath,
  expanded,
  loadingByPath,
  loadingRoot,
  pruneDeadDir,
  rootEntry,
  rootError,
  searchActive,
  selectedPath,
} from "./state";
import { clearSearch, runSearchNow } from "./search";

/** 兜底：从后端直接取启动工作目录（store 尚未就绪时用） */
export async function resolveFallbackRoot(): Promise<string> {
  try {
    const w = await invoke<string>("startup_workspace");
    if (w && w.trim()) return normalizeFsPath(w.trim());
  } catch {
    // 忽略，沿用空根
  }
  return "";
}

/** 拉取一个目录的直接子项（懒加载；已缓存且非强制时直接返回）。
 *  `opts.silent` 用于程序化/自动刷新：列表失败时静默剪枝，不打扰用户。 */
export async function loadDir(
  path: string,
  force = false,
  opts: { silent?: boolean } = {},
): Promise<void> {
  const root = normalizeFsPath(workspace.value);
  if (!root) return;
  if (!force && childrenByPath[path] !== undefined) return;
  loadingByPath[path] = true;
  try {
    childrenByPath[path] = await invoke<FsEntry[]>("session_fs_list", {
      workspace: root,
      dir: path,
    });
  } catch (e) {
    // 目录已不可列（被删除/改名等）：从缓存剪枝，避免后续刷新反复重列并弹错；
    // 自动刷新（silent）静默，用户主动操作（展开/刷新按钮）仍提示一次。
    pruneDeadDir(path);
    if (!opts.silent) setToast(toastError(e));
  } finally {
    loadingByPath[path] = false;
  }
}

/** 加载根节点元信息 + 第一层子项（root 为已解析的绝对路径）；
 * 返回是否成功写入（请求期间工作区已切换时结果作废返回 false） */
export async function loadRoot(root: string): Promise<boolean> {
  loadingRoot.value = true;
  rootError.value = "";
  try {
    const entry = await invoke<FsEntry>("session_fs_metadata", {
      workspace: root,
      path: root,
    });
    if (!pathEquals(workspace.value, root)) return false; // 请求期间工作区已切换：结果作废
    rootEntry.value = entry;
    expanded.add(root);
    await loadDir(root);
    return true;
  } catch (e) {
    rootError.value = toastError(e);
    return false;
  } finally {
    loadingRoot.value = false;
  }
}

let rootLoadInFlight: Promise<boolean> | null = null;
let rootLoadTarget = "";

/** 确保根已加载（单飞）：已就绪直接返回；同根并发加载共享同一次请求；
 * 供 revealAbsPath 与资源面板激活并行时去重，避免重复拉根元信息 */
export function ensureRootLoaded(root: string): Promise<boolean> {
  if (rootEntry.value?.path === root && childrenByPath[root] !== undefined) {
    return Promise.resolve(true);
  }
  if (rootLoadInFlight && rootLoadTarget === root) {
    return rootLoadInFlight;
  }
  rootLoadTarget = root;
  rootLoadInFlight = loadRoot(root).finally(() => {
    rootLoadInFlight = null;
    rootLoadTarget = "";
  });
  return rootLoadInFlight;
}

/** 展开/折叠目录；首次展开时懒加载子项 */
export function toggleDir(path: string) {
  if (expanded.has(path)) expanded.delete(path);
  else {
    expanded.add(path);
    void loadDir(path);
  }
}

/** 刷新根 + 所有已加载目录，保留展开状态 */
export async function refreshAll() {
  const root = normalizeFsPath(workspace.value);
  if (!root) return;
  if (searchActive.value) {
    await runSearchNow();
    return;
  }
  loadingRoot.value = true;
  try {
    const entry = await invoke<FsEntry>("session_fs_metadata", {
      workspace: root,
      path: root,
    });
    if (workspace.value !== root) return; // 请求期间工作区已切换：结果作废
    rootEntry.value = entry;
    expanded.add(root);
  } catch (e) {
    if (workspace.value !== root) return;
    rootError.value = toastError(e);
  } finally {
    loadingRoot.value = false;
  }
  const paths = [
    root,
    ...Object.keys(childrenByPath).filter((p) => isPathUnderRoot(root, p)),
  ];
  for (const p of paths) {
    await loadDir(p, true, { silent: true });
  }
}

/** 按 data-fs-path 大小写不敏感查找资源树行（Windows 路径大小写不敏感） */
function findRowByPath(path: string): HTMLElement | null {
  for (const el of document.querySelectorAll<HTMLElement>("[data-fs-path]")) {
    const p = el.dataset.fsPath;
    if (p && pathEquals(p, path)) return el;
  }
  return null;
}

/**
 * 等待目标行渲染后滚动定位：面板刚激活/目录异步加载时行可能晚于 selectedPath
 * 落定才出现，且 git 路径与磁盘路径可能大小写不一致；找到后把 selectedPath 校正为
 * 树的规范路径（保证高亮匹配），再 scrollIntoView。上限重试，超时静默放弃
 * （已删除文件等不在树中的路径）。
 */
async function scrollToReveal(path: string): Promise<void> {
  const MAX_TRIES = 12;
  for (let i = 0; i < MAX_TRIES; i++) {
    await nextTick();
    const el = findRowByPath(path);
    if (!el) continue;
    const canonical = el.dataset.fsPath;
    if (canonical && canonical !== path) {
      selectedPath.value = canonical;
    }
    el.scrollIntoView({ block: "center" });
    return;
  }
}

/**
 * 按绝对路径在树中定位：路径不在当前工作区内（大小写不敏感边界判定）视为
 * 「匹配不上」直接跳过；匹配时逐级展开祖先目录、清除搜索、选中并滚动到可见。
 * expandTarget=true 时目标目录自身也展开（搜索结果点击目录用）。
 */
async function revealAbsPath(
  absPath: string,
  expandTarget: boolean,
): Promise<void> {
  const root = normalizeFsPath(workspace.value);
  if (!root || !absPath) return;
  const norm = normalizeFsPath(absPath);
  if (!isPathUnderRoot(root, norm)) return;
  // 根尚未加载（资源面板从未激活/工作区刚切换）时先加载根，否则下方祖先展开后
  // 树仍无法渲染目标行，selectedPath/scrollIntoView 会静默失效（diff 标签联动资源树失效）
  await ensureRootLoaded(root);
  const parts = relPathOf(root, norm)
    .replace(/\\/g, "/")
    .split("/")
    .filter(Boolean);
  const dirs = expandTarget ? parts : parts.slice(0, -1);
  let cur = root;
  for (const part of dirs) {
    cur = joinFsPath(cur, part);
    expanded.add(cur);
    await loadDir(cur, false, { silent: true });
  }
  selectedPath.value = norm;
  clearSearch();
  await scrollToReveal(norm);
}

/** Tab 激活同步入口：文件/预览/Diff 标签带绝对路径时调用；工作区外路径自动跳过 */
export async function revealAbsPathInTree(absPath: string): Promise<void> {
  await revealAbsPath(absPath, false);
}

/** 面板切换同步入口：活动标签为 文件/diff/预览 时在资源树中定位（终端/会话标签不定位） */
export async function revealActiveTab(): Promise<void> {
  const tab = activeTab.value;
  if (
    !tab ||
    tab.kind === TabKind.Terminal ||
    tab.kind === TabKind.Chat ||
    tab.kind === TabKind.Commit ||
    tab.kind === TabKind.Settings
  ) {
    return;
  }
  const absPath = /^[A-Za-z]:[\\/]/.test(tab.path)
    ? tab.path
    : joinFsPath(tab.workspace, tab.path);
  await revealAbsPathInTree(absPath);
}

/** 点击搜索结果：展开祖先目录、清除搜索、选中并滚动到树中该行 */
export async function revealInTree(entry: FsEntry) {
  await revealAbsPath(entry.path, entry.isDir);
}
