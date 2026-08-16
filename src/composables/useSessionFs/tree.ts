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
import {
  childrenByPath,
  expanded,
  loadingByPath,
  loadingRoot,
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
    if (w && w.trim()) return w.trim();
  } catch {
    // 忽略，沿用空根
  }
  return "";
}

/** 拉取一个目录的直接子项（懒加载；已缓存且非强制时直接返回） */
export async function loadDir(path: string, force = false): Promise<void> {
  const root = workspace.value;
  if (!root) return;
  if (!force && childrenByPath[path] !== undefined) return;
  loadingByPath[path] = true;
  try {
    childrenByPath[path] = await invoke<FsEntry[]>("session_fs_list", {
      workspace: root,
      dir: path,
    });
  } catch (e) {
    if (childrenByPath[path] === undefined) childrenByPath[path] = [];
    setToast(toastError(e));
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
    if (workspace.value !== root) return false; // 请求期间工作区已切换：结果作废
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
  const root = workspace.value;
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
    await loadDir(p, true);
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
  const root = workspace.value;
  if (!root || !absPath) return;
  const norm = absPath.replace(/\//g, "\\");
  if (!isPathUnderRoot(root, norm)) return;
  const parts = relPathOf(root, norm)
    .replace(/\\/g, "/")
    .split("/")
    .filter(Boolean);
  const dirs = expandTarget ? parts : parts.slice(0, -1);
  let cur = root;
  for (const part of dirs) {
    cur = joinFsPath(cur, part);
    expanded.add(cur);
    await loadDir(cur);
  }
  selectedPath.value = norm;
  clearSearch();
  await nextTick();
  document
    .querySelector(`[data-fs-path="${CSS.escape(norm)}"]`)
    ?.scrollIntoView({ block: "center" });
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
    tab.kind === TabKind.Chat
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
