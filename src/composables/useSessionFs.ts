import { computed, nextTick, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { setToast, store, toastError, workspace } from "./useCodex";
import { activeTab, openFileTab, openPreviewTab } from "./useEditorTabs";
import { TabKind } from "../lib/tabs";
import { toUserAttachment } from "../lib/mention";
import type { UserInput } from "../lib/types";
import { debounce } from "../lib/debounce";
import { pathBaseName, relPathOf } from "../lib/format";
import { previewTypeForName } from "../lib/preview";
import {
  flattenResourceTree,
  joinFsPath,
  type FsEntry,
  type IconRequest,
  type IconResult,
  type ResourceRow,
} from "../lib/sessionFs";

const SEARCH_LIMIT = 200;
/** 图标缓存上限：超限按插入顺序淘汰最旧 */
const ICON_CACHE_MAX = 1000;

export const rootEntry = ref<FsEntry | null>(null);
export const rootError = ref("");
export const loadingRoot = ref(false);
/** 目录路径 → 已加载的直接子项（懒加载缓存） */
export const childrenByPath = reactive<Record<string, FsEntry[]>>({});
/** 展开中的目录路径集合（含根） */
export const expanded = reactive(new Set<string>());
export const loadingByPath = reactive<Record<string, boolean>>({});

export const searchTerm = ref("");
export const searchResults = ref<FsEntry[]>([]);
export const searching = ref(false);
export const selectedPath = ref("");
/** 内部复制记录：粘贴时优先使用，为空回退系统剪贴板 */
export const copyBuffer = ref<string[]>([]);
/**
 * 文件类型图标缓存：键为 `ext:<小写扩展名>`（如 ext:.rs）或无扩展名文件的
 * `file:<relPath>`；值为 PNG data URI，null 表示取不到（不再重试）。
 * 目录不在范围，继续使用内置 SVG 文件夹图标。
 */
export const iconCache = reactive(new Map<string, string | null>());

export const searchActive = computed(() => searchTerm.value.trim().length > 0);

export const treeRows = computed<ResourceRow[]>(() => {
  if (!rootEntry.value) return [];
  return flattenResourceTree(rootEntry.value, childrenByPath, expanded);
});

let active = false;
/** 已加载的根路径：同根重新激活时保留展开状态，仅刷新数据 */
let loadedRoot = "";
let searchSeq = 0;
let unlistenFsEvent: UnlistenFn | null = null;
let watcherStarted = false;

const debouncedSearch = debounce(() => void runSearchNow(), 300);

function resetTree() {
  for (const k of Object.keys(childrenByPath)) delete childrenByPath[k];
  for (const k of Object.keys(loadingByPath)) delete loadingByPath[k];
  expanded.clear();
  rootEntry.value = null;
  rootError.value = "";
  searchResults.value = [];
  selectedPath.value = "";
  copyBuffer.value = [];
  iconCache.clear();
}

/** 图标缓存键：目录返回 null（不在范围）；文件按扩展名/无扩展名路径 */
export function iconCacheKey(entry: FsEntry): string | null {
  if (entry.isDir) return null;
  const dot = entry.name.lastIndexOf(".");
  if (dot > 0 && dot < entry.name.length - 1) {
    return `ext:${entry.name.slice(dot).toLowerCase()}`;
  }
  return `file:${entry.relPath}`;
}

/** 读取缓存图标：未缓存或目录返回 null（渲染层回退 SVG） */
export function iconFor(entry: FsEntry): string | null {
  const key = iconCacheKey(entry);
  return key ? (iconCache.get(key) ?? null) : null;
}

function setIconCache(key: string, value: string | null) {
  iconCache.delete(key); // 重新插入，保持 LRU 顺序
  iconCache.set(key, value);
  if (iconCache.size > ICON_CACHE_MAX) {
    const oldest = iconCache.keys().next().value;
    if (oldest !== undefined) iconCache.delete(oldest);
  }
}

/**
 * 按可见行懒加载缺失的文件图标：同扩展名只发一个代表路径，结果回填缓存；
 * 单个失败缓存 null 不重试；整批失败（如 root 无效）不缓存，待下次可见行变化重试。
 */
export async function ensureEntryIcons(
  entries: FsEntry[],
  root: string = workspace.value,
): Promise<void> {
  if (!root || !entries.length) return;
  const byKey = new Map<string, FsEntry>();
  for (const e of entries) {
    const key = iconCacheKey(e);
    if (key && !iconCache.has(key) && !byKey.has(key)) byKey.set(key, e);
  }
  if (!byKey.size) return;

  const pathToKey = new Map<string, string>();
  const requests: IconRequest[] = [];
  for (const e of byKey.values()) {
    pathToKey.set(e.path, iconCacheKey(e)!);
    requests.push({ path: e.path });
  }
  try {
    const results = await invoke<IconResult[]>("session_fs_icons", { workspace: root, requests });
    for (const r of results) {
      const key = pathToKey.get(r.path);
      if (key) setIconCache(key, r.dataUri);
    }
    // 请求了但未返回的键置 null，避免反复请求
    for (const key of byKey.keys()) {
      if (!iconCache.has(key)) setIconCache(key, null);
    }
  } catch {
    // 整批失败：保持未缓存状态，稍后重试
  }
}

/** 「新建文本文件」菜单系统图标任务：防止重复发起取图请求 */
let textFileIconTask: Promise<void> | null = null;

/** 按 .txt 扩展名取一次系统图标并写入缓存（复用 ext:.txt 键，.txt 文件行/标签同步受益） */
export async function ensureTextFileIcon(): Promise<void> {
  const root = workspace.value;
  if (!root || iconCache.has("ext:.txt")) return;
  if (textFileIconTask) return textFileIconTask;
  textFileIconTask = (async () => {
    try {
      const uri = await invoke<string | null>("session_fs_icon_for_ext", {
        ext: ".txt",
      });
      setIconCache("ext:.txt", uri ?? null);
    } catch {
      // 取不到/失败：缓存 null 不再重试，菜单回退内置 SVG
      setIconCache("ext:.txt", null);
    } finally {
      textFileIconTask = null;
    }
  })();
  return textFileIconTask;
}

/** 右键菜单「新建文本文件」图标：缓存未就绪/不可用时返回 undefined（调用方回退 SVG） */
export function textFileMenuIcon(): string | undefined {
  return iconCache.get("ext:.txt") ?? undefined;
}

/** 兜底：从后端直接取启动工作目录（store 尚未就绪时用） */
async function resolveFallbackRoot(): Promise<string> {
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

/** 加载根节点元信息 + 第一层子项（root 为已解析的绝对路径） */
async function loadRoot(root: string) {
  loadingRoot.value = true;
  rootError.value = "";
  try {
    rootEntry.value = await invoke<FsEntry>("session_fs_metadata", {
      workspace: root,
      path: root,
    });
    expanded.add(root);
    await loadDir(root);
  } catch (e) {
    rootError.value = toastError(e);
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
    rootEntry.value = await invoke<FsEntry>("session_fs_metadata", {
      workspace: root,
      path: root,
    });
    expanded.add(root);
  } catch (e) {
    rootError.value = toastError(e);
  } finally {
    loadingRoot.value = false;
  }
  const paths = [root, ...Object.keys(childrenByPath)];
  for (const p of paths) {
    await loadDir(p, true);
  }
}

export function onSearchInput() {
  debouncedSearch.run();
}

export async function runSearchNow() {
  const q = searchTerm.value.trim();
  if (!q) {
    searchSeq++;
    searchResults.value = [];
    searching.value = false;
    return;
  }
  const root = workspace.value;
  if (!root) return;
  const seq = ++searchSeq;
  searching.value = true;
  try {
    const res = await invoke<FsEntry[]>("session_fs_search", {
      workspace: root,
      query: q,
      limit: SEARCH_LIMIT,
    });
    if (seq === searchSeq) searchResults.value = res;
  } catch (e) {
    if (seq === searchSeq) {
      searchResults.value = [];
      setToast(toastError(e));
    }
  } finally {
    if (seq === searchSeq) searching.value = false;
  }
}

export function clearSearch() {
  searchTerm.value = "";
  debouncedSearch.cancel();
  searchSeq++;
  searchResults.value = [];
  searching.value = false;
}

/**
 * 按绝对路径在树中定位：路径不在当前工作区内（大小写不敏感边界判定）视为
 * 「匹配不上」直接跳过；匹配时逐级展开祖先目录、清除搜索、选中并滚动到可见。
 * expandTarget=true 时目标目录自身也展开（搜索结果点击目录用）。
 */
async function revealAbsPath(absPath: string, expandTarget: boolean): Promise<void> {
  const root = workspace.value;
  if (!root || !absPath) return;
  const norm = absPath.replace(/\//g, "\\");
  if (!isPathUnderRoot(root, norm)) return;
  const parts = relPathOf(root, norm).replace(/\\/g, "/").split("/").filter(Boolean);
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
  if (!tab || tab.kind === TabKind.Terminal) return;
  // 会话标签不参与 activeTab（会话标签由 sessionTabs 管理），此处只剩 文件/diff/预览
  const absPath = /^[A-Za-z]:[\\/]/.test(tab.path)
    ? tab.path
    : joinFsPath(tab.workspace, tab.path);
  await revealAbsPathInTree(absPath);
}

/** 点击搜索结果：展开祖先目录、清除搜索、选中并滚动到树中该行 */
export async function revealInTree(entry: FsEntry) {
  await revealAbsPath(entry.path, entry.isDir);
}

export function copyEntry(entry: FsEntry) {
  copyBuffer.value = [entry.path];
  setToast(`已复制「${entry.name}」，可在目录上右键粘贴`);
}

async function readClipboardPaths(): Promise<string[]> {
  try {
    return await invoke<string[]>("clipboard_file_paths");
  } catch {
    return [];
  }
}

/** 把内部复制记录（或系统剪贴板文件）粘贴进目标目录 */
export async function pasteInto(targetDir: string) {
  const root = workspace.value;
  if (!root) return;
  const sources = copyBuffer.value.length
    ? [...copyBuffer.value]
    : await readClipboardPaths();
  if (!sources.length) {
    setToast("剪贴板中没有可粘贴的文件或文件夹");
    return;
  }
  try {
    const created = await invoke<FsEntry[]>("session_fs_paste", {
      workspace: root,
      destDir: targetDir,
      sources,
    });
    copyBuffer.value = [];
    setToast(`已粘贴 ${created.length} 项`);
    await refreshAll();
  } catch (e) {
    setToast(toastError(e));
  }
}

/** 粘贴是否可用：内部复制记录非空或系统剪贴板含文件（读取失败按不可用处理） */
export async function pasteAvailable(): Promise<boolean> {
  if (copyBuffer.value.length) return true;
  const paths = await readClipboardPaths();
  return paths.length > 0;
}

/** 在目录下新建文本文件（唯一命名由后端保证），成功后刷新并返回条目；失败返回 null */
export async function createTextFile(dir: string): Promise<FsEntry | null> {
  const root = workspace.value;
  if (!root) return null;
  try {
    const created = await invoke<FsEntry>("session_fs_create_file", {
      workspace: root,
      dir,
    });
    setToast(`已创建「${created.name}」`);
    await refreshAll();
    return created;
  } catch (e) {
    setToast(toastError(e));
    return null;
  }
}

/** 在目录下新建文件夹（唯一命名由后端保证），成功后刷新并返回条目；失败返回 null */
export async function createFolder(dir: string): Promise<FsEntry | null> {
  const root = workspace.value;
  if (!root) return null;
  try {
    const created = await invoke<FsEntry>("session_fs_create_dir", {
      workspace: root,
      dir,
    });
    setToast(`已创建「${created.name}」`);
    await refreshAll();
    return created;
  } catch (e) {
    setToast(toastError(e));
    return null;
  }
}

export async function renameEntry(path: string, newName: string) {
  const root = workspace.value;
  if (!root) return;
  try {
    await invoke<FsEntry>("session_fs_rename", { workspace: root, path, newName });
    await refreshAll();
  } catch (e) {
    setToast(toastError(e));
  }
}

export async function deleteEntry(path: string) {
  const root = workspace.value;
  if (!root) return;
  try {
    await invoke("session_fs_delete", { workspace: root, path });
    await refreshAll();
  } catch (e) {
    setToast(toastError(e));
  }
}

/** 树内拖拽移动：把 src 移动到 destDir 目录下；成功后刷新并提示，返回是否成功 */
export async function moveEntry(src: string, destDir: string): Promise<boolean> {
  const root = workspace.value;
  if (!root) return false;
  try {
    await invoke<FsEntry>("session_fs_move", { workspace: root, src, destDir });
    const name = src.split(/[\\/]/).pop() ?? src;
    setToast(`已移动「${name}」`);
    await refreshAll();
    return true;
  } catch (e) {
    setToast(toastError(e));
    return false;
  }
}

export function revealInExplorer(path: string) {
  void invoke("reveal_path", { path }).catch((e) => setToast(toastError(e)));
}

/** 应用内打开文本文件：在主窗口左侧编辑器区打开/激活一个文件标签 */
export function openTextEditor(entry: FsEntry) {
  const root = workspace.value;
  if (!root) return;
  void openFileTab(root, entry.path);
}

/** 应用内打开 PDF 预览：在主窗口左侧编辑器区打开/激活 PDF 预览标签 */
export function openPdfPreview(entry: FsEntry) {
  const root = workspace.value;
  if (!root) return;
  void openPreviewTab("pdf", root, entry.path);
}

/** 应用内打开图像预览：在主窗口左侧编辑器区打开/激活图像预览标签 */
export function openImagePreview(entry: FsEntry) {
  const root = workspace.value;
  if (!root) return;
  void openPreviewTab("image", root, entry.path);
}

/**
 * 打开前探测文件内容是否为文本：true=文本、false=二进制/非文本、
 * null=探测失败（已 toast 错误；无工作目录时静默返回 null）
 */
export async function probeTextEntry(
  entry: FsEntry,
): Promise<boolean | null> {
  const root = workspace.value;
  if (!root) return null;
  try {
    return await invoke<boolean>("session_fs_probe_text", {
      workspace: root,
      path: entry.path,
    });
  } catch (e) {
    setToast(toastError(e));
    return null;
  }
}

/** Windows 路径 dirname：取最后一个分隔符前的部分（去尾分隔符）；无分隔符返回空串 */
function dirNameOf(path: string): string {
  const norm = path.replace(/\//g, "\\");
  const idx = norm.lastIndexOf("\\");
  if (idx < 0) return "";
  const dir = norm.slice(0, idx).replace(/\\+$/, "");
  return /^[A-Za-z]:$/.test(dir) ? dir + "\\" : dir;
}

/** 路径是否位于根目录之内（Windows 大小写不敏感，按分隔符边界判定） */
function isPathUnderRoot(root: string, path: string): boolean {
  const a = root.replace(/\//g, "\\").toLowerCase().replace(/\\+$/, "");
  const b = path.replace(/\//g, "\\").toLowerCase();
  if (!a) return false;
  return b === a || b.startsWith(a + "\\");
}

/**
 * 对话本地链接：支持则在应用内 tab 打开（PDF/图片 → 预览标签，文本 → 编辑器），
 * 返回 true；否则返回 false，由调用方降级为资源管理器。
 * 工作区外文件以父目录作为根（仅本次读取/打开，不改变会话工作区）。
 * 测试钩子（__CODEX_UI_TEST__）开启时直接返回 false，保持 E2E 现有
 * reveal_path 分发记录不回归。
 */
export async function openPathInApp(path: string): Promise<boolean> {
  const testWin = window as unknown as { __CODEX_UI_TEST__?: boolean };
  if (testWin.__CODEX_UI_TEST__) return false;
  const session = workspace.value;
  if (!session) return false;
  const underSession = isPathUnderRoot(session, path);
  const root = underSession ? session : dirNameOf(path);
  const relPath = underSession ? path : pathBaseName(path);
  if (!root || !relPath) return false;

  const type = previewTypeForName(relPath);
  if (type === "pdf") {
    void openPreviewTab("pdf", root, relPath);
    return true;
  }
  if (type === "image") {
    // 图片走 asset 协议，需要绝对路径；root 仅作标签元数据
    void openPreviewTab("image", root, path);
    return true;
  }
  try {
    const isText = await invoke<boolean>("session_fs_probe_text", {
      workspace: root,
      path: relPath,
    });
    if (!isText) return false;
    void openFileTab(root, relPath);
    return true;
  } catch {
    // 目录/缺失/不可读：降级为资源管理器（原行为），不额外 toast
    return false;
  }
}

/** 添加为会话附件：优先走 ComposerBar 全局入口，缺失时兜底 push store */
export function addAsAttachment(entry: FsEntry) {
  const a = toUserAttachment(entry.name, entry.path);
  const w = window as unknown as {
    __CODEX_UI_ADD_ATTACHMENT__?: (a: UserInput) => void;
  };
  if (typeof w.__CODEX_UI_ADD_ATTACHMENT__ === "function") {
    w.__CODEX_UI_ADD_ATTACHMENT__(a);
  } else {
    store.attachments.push(a);
  }
  setToast(`已添加「${entry.name}」为会话附件`);
}

async function syncWatcher() {
  const root = workspace.value;
  if (active && root) {
    if (!unlistenFsEvent) {
      try {
        unlistenFsEvent = await listen("session-fs/changed", () => {
          if (active) void refreshAll();
        });
      } catch {
        // 非 Tauri 环境（如单测）忽略
      }
    }
    try {
      await invoke("session_fs_watch_start", { workspace: root });
      watcherStarted = true;
    } catch (e) {
      setToast(toastError(e));
    }
  } else {
    unlistenFsEvent?.();
    unlistenFsEvent = null;
    if (watcherStarted) {
      try {
        await invoke("session_fs_watch_stop");
      } catch {
        // 忽略停止失败
      }
      watcherStarted = false;
    }
  }
}

/** 激活资源 Tab：解析工作区（含 startup_workspace 兜底）后加载树 */
async function activate() {
  const root = workspace.value || (await resolveFallbackRoot());
  if (!root) {
    rootError.value = "暂无工作目录";
    return;
  }
  if (!workspace.value) {
    // 兜底结果回写全局，让头部等其它读取点保持一致
    store.server.startupWorkspace = root;
  }
  // 预取「新建文本文件」菜单的系统 .txt 图标（写 ext:.txt 缓存）
  void ensureTextFileIcon();
  if (loadedRoot !== root) {
    loadedRoot = root;
    resetTree();
    await loadRoot(root);
  } else {
    await refreshAll();
  }
}

/** 会话资源 Tab 激活状态：激活时启动监听并加载树，切走时停止监听 */
export function setSessionFsActive(v: boolean) {
  if (active === v) return;
  active = v;
  void syncWatcher();
  if (v) void activate();
}

// 根目录切换（切换会话/新建会话选目录）：重置并重新加载，监听跟随新根
watch(workspace, (r, old) => {
  if (r === old) return;
  if (!active) return;
  if (r) {
    if (loadedRoot !== r) {
      loadedRoot = r;
      resetTree();
      void loadRoot(r);
    }
  } else {
    resetTree();
    rootError.value = "暂无工作目录";
  }
  void syncWatcher();
});

/** 仅测试用：清空模块状态 */
export function __resetSessionFsForTest() {
  active = false;
  loadedRoot = "";
  resetTree();
  searchTerm.value = "";
  searching.value = false;
  debouncedSearch.cancel();
  searchSeq++;
  unlistenFsEvent?.();
  unlistenFsEvent = null;
  watcherStarted = false;
}
