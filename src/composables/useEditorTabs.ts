import { computed, markRaw, reactive, ref, shallowReactive } from "vue";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { Compartment, EditorState, Text } from "@codemirror/state";
import {
  buildSaveContent,
  detectEol,
  normalizeForEditor,
  stripBom,
  type EditorEol,
} from "../lib/editorFile";
import { pathBaseName } from "../lib/format";
import { base64ToBytes, type PreviewType } from "../lib/preview";
import type { DiffRow } from "../lib/types";
import {
  attachTerminal,
  ensureTerminalListeners,
  releaseTerminal,
} from "./useTerminalEvents";

/** 会话文件读取结果（与 Rust session_fs_read 返回结构一致） */
interface TextFileContent {
  content: string;
  validUtf8: boolean;
  byteSize: number;
}

/** 会话二进制文件读取结果（与 Rust session_fs_read_bytes 返回结构一致） */
interface BinaryFileContent {
  content: string;
  byteSize: number;
}

/** diff 预览参数（与 Rust DiffPreviewParams 结构一致） */
export interface DiffPreviewParams {
  path: string;
  kind: string;
  diff: string;
  workspace_root: string;
}

export interface ChatEditorTab {
  kind: "chat";
  id: "chat";
  title: string;
  closable: false;
}

export interface FileEditorTab {
  kind: "file";
  id: string;
  root: string;
  path: string;
  title: string;
  loading: boolean;
  error: string;
  readOnly: boolean;
  dirty: boolean;
  saving: boolean;
  wrap: boolean;
  eol: EditorEol;
  hadBom: boolean;
  byteSize: number | null;
  cursor: { line: number; col: number };
  status: string;
  /** CodeMirror 状态（非响应式，避免深度代理开销）；切换标签时由编辑组件 setState */
  editorState: EditorState | null;
  savedText: Text | null;
  wrapCompartment: Compartment | null;
}

export interface DiffEditorTab {
  kind: "diff";
  id: string;
  path: string;
  /** diff 变化类型：add / delete / modify */
  changeKind: string;
  workspaceRoot: string;
  title: string;
  loading: boolean;
  error: string;
  rows: DiffRow[];
  /** 原始 unified diff，行解析失败时回退展示 */
  fallback: string;
  /** 简要模式：仅显示变更行（隐藏未变化上下文） */
  brief: boolean;
}

export interface PreviewEditorTab {
  kind: "preview";
  /** 预览类型：pdf → pdf.js 渲染；image → asset URL 直显 */
  previewType: PreviewType;
  id: string;
  root: string;
  path: string;
  title: string;
  loading: boolean;
  error: string;
  /** 图像预览：convertFileSrc(path) 的 asset URL */
  imageUrl: string;
  /** PDF 预览：后端读取的原始字节（pdf.js getDocument 数据源） */
  pdfData: Uint8Array | null;
  /** PDF 页数：组件加载文档后回填 */
  pageCount: number | null;
}

export interface TerminalEditorTab {
  kind: "terminal";
  id: string;
  /** 终端启动目录（绝对路径） */
  cwd: string;
  title: string;
  loading: boolean;
  error: string;
  /** 进程已退出（收到 terminal/exit 事件后置位） */
  exited: boolean;
  exitCode: number | null;
}

export type EditorTab =
  | ChatEditorTab
  | FileEditorTab
  | DiffEditorTab
  | PreviewEditorTab
  | TerminalEditorTab;

/** 标签列表：第一个固定为“对话”主标签，不可关闭 */
export const tabs = shallowReactive<EditorTab[]>([]);
export const activeTabId = ref("chat");
/** 待关闭的脏文件标签 id（由编辑面板弹确认层） */
export const pendingCloseId = ref<string | null>(null);

export const activeTab = computed<EditorTab | null>(
  () => tabs.find((t) => t.id === activeTabId.value) ?? null,
);

function initChatTab() {
  if (!tabs.some((t) => t.id === "chat")) {
    tabs.unshift({ kind: "chat", id: "chat", title: "对话", closable: false });
  }
}
initChatTab();

function fileTabId(root: string, path: string): string {
  return "file:" + JSON.stringify([root, path]);
}

function diffTabId(p: DiffPreviewParams): string {
  return "diff:" + JSON.stringify([p.workspace_root, p.path, p.kind]);
}

function previewTabId(type: PreviewType, root: string, path: string): string {
  return `preview:${type}:${JSON.stringify([root, path])}`;
}

function nowTime(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

/** 终端标签自增序号：保证同一毫秒内连续多开也生成不同 id */
let terminalSeq = 0;

/**
 * 打开终端标签：每次调用都新建独立会话（支持同目录多开），生成唯一 id、
 * 激活标签后向后端发起 terminal_spawn；失败保留标签并记录错误。
 */
export async function openTerminalTab(cwd: string): Promise<void> {
  const id = `terminal:${++terminalSeq}:${Date.now()}`;
  const tab = reactive({
    kind: "terminal",
    id,
    cwd,
    title: pathBaseName(cwd) || cwd,
    loading: true,
    error: "",
    exited: false,
    exitCode: null,
  }) as unknown as TerminalEditorTab;
  tabs.push(tab);
  activeTabId.value = id;
  try {
    // 先注册全局事件监听并建立缓冲，再 spawn，避免启动输出（含 ConPTY DSR
    // 查询）在懒加载面板挂载前丢失导致首个终端空白。
    await ensureTerminalListeners();
    attachTerminal(id);
    await invoke("terminal_spawn", { id, cwd });
  } catch (e) {
    tab.error = String(e);
  } finally {
    tab.loading = false;
    // 启动期间标签已被关闭（closeTab 先执行）：spawn 完成后回收后端会话，
    // 避免遗留无人引用的 PTY 进程
    if (!tabs.some((t) => t.id === id)) {
      void invoke("terminal_kill", { id }).catch(() => {});
    }
  }
}

export function activateTab(id: string): void {
  if (tabs.some((t) => t.id === id)) activeTabId.value = id;
}

/**
 * 打开文本文件标签：已打开则直接激活；否则新建标签并异步读取内容、
 * 构建 CodeMirror 状态（按标签闭包维护 dirty/cursor/最新 state）。
 */
export async function openFileTab(root: string, path: string): Promise<void> {
  const id = fileTabId(root, path);
  if (tabs.some((t) => t.id === id)) {
    activeTabId.value = id;
    return;
  }
  // reactive 的类型会深展开 EditorState，这里用断言还原为标签类型；
  // 运行时编辑器状态在赋值时均已 markRaw，不会被深度代理。
  const tab = reactive({
    kind: "file",
    id,
    root,
    path,
    title: pathBaseName(path) || path,
    loading: true,
    error: "",
    readOnly: false,
    dirty: false,
    saving: false,
    wrap: false,
    eol: "\n",
    hadBom: false,
    byteSize: null,
    cursor: { line: 1, col: 1 },
    status: "",
    editorState: null,
    savedText: null,
    wrapCompartment: null,
  }) as unknown as FileEditorTab;
  tabs.push(tab);
  activeTabId.value = id;
  try {
    const info = await invoke<TextFileContent>("session_fs_read", {
      root,
      path,
    });
    const ro = !info.validUtf8;
    const { text, hadBom } = stripBom(info.content);
    const lineEol = detectEol(text);
    const doc = normalizeForEditor(text, lineEol);
    // CodeMirror 相关模块仅在首次打开文件时加载（保持主包轻量）
    const [{ buildEditorExtensions, createEditorState, languageForPath }, { Compartment }] =
      await Promise.all([
        import("../lib/editorSetup"),
        import("@codemirror/state"),
      ]);
    const wrapCompartment = new Compartment();
    const state = createEditorState(
      doc,
      buildEditorExtensions({
        language: languageForPath(path),
        readOnly: ro,
        wrap: false,
        wrapCompartment,
        savedText: () => tab.savedText,
        onDirtyChange: (d) => {
          tab.dirty = d;
        },
        onCursorChange: (line, col) => {
          tab.cursor = { line, col };
        },
        onSave: () => {
          void saveFileTab(tab.id);
        },
        onStateChange: (s) => {
          tab.editorState = markRaw(s);
        },
      }),
    );
    tab.readOnly = ro;
    tab.eol = lineEol;
    tab.hadBom = hadBom;
    tab.byteSize = info.byteSize;
    tab.wrapCompartment = markRaw(wrapCompartment);
    tab.editorState = markRaw(state);
    tab.savedText = markRaw(state.doc);
    if (ro) {
      tab.status = "文件不是 UTF-8 编码，已以只读方式打开";
    }
  } catch (e) {
    tab.error = String(e);
  } finally {
    tab.loading = false;
  }
}

/** 保存指定文件标签；成功返回 true 并复位脏标记 */
export async function saveFileTab(id: string): Promise<boolean> {
  const tab = tabs.find(
    (t): t is FileEditorTab => t.kind === "file" && t.id === id,
  );
  if (!tab || !tab.editorState || tab.readOnly || !tab.dirty || tab.saving) {
    return false;
  }
  tab.saving = true;
  try {
    const content = buildSaveContent(
      tab.editorState.doc.toString(),
      tab.eol,
      tab.hadBom,
    );
    await invoke("session_fs_write", {
      root: tab.root,
      path: tab.path,
      content,
    });
    tab.savedText = markRaw(tab.editorState.doc);
    tab.dirty = false;
    tab.status = `已保存 ${nowTime()}`;
    return true;
  } catch (e) {
    tab.status = `保存失败：${String(e)}`;
    return false;
  } finally {
    tab.saving = false;
  }
}

/** 打开 diff 预览标签：已打开则激活；否则取行数据后展示 */
export async function openDiffTab(params: DiffPreviewParams): Promise<void> {
  const id = diffTabId(params);
  if (tabs.some((t) => t.id === id)) {
    activeTabId.value = id;
    return;
  }
  const tab = reactive({
    kind: "diff",
    id,
    path: params.path,
    changeKind: params.kind,
    workspaceRoot: params.workspace_root,
    title: pathBaseName(params.path) || params.path,
    loading: true,
    error: "",
    rows: [],
    fallback: params.diff,
    brief: false,
  }) as unknown as DiffEditorTab;
  tabs.push(tab);
  activeTabId.value = id;
  try {
    const rows = await invoke<DiffRow[]>("build_diff_preview", { params });
    tab.rows = rows ?? [];
  } catch (e) {
    tab.error = String(e);
  } finally {
    tab.loading = false;
  }
}

/**
 * 打开特殊文件预览标签（PDF / 图像）：已打开则激活；否则新建标签并异步准备数据。
 * 图像直接经 asset 协议取 URL；PDF 经 session_fs_read_bytes 读取 base64 后解码为字节。
 */
export async function openPreviewTab(
  type: PreviewType,
  root: string,
  path: string,
): Promise<void> {
  const id = previewTabId(type, root, path);
  if (tabs.some((t) => t.id === id)) {
    activeTabId.value = id;
    return;
  }
  const tab = reactive({
    kind: "preview",
    previewType: type,
    id,
    root,
    path,
    title: pathBaseName(path) || path,
    loading: true,
    error: "",
    imageUrl: "",
    pdfData: null,
    pageCount: null,
  }) as unknown as PreviewEditorTab;
  tabs.push(tab);
  activeTabId.value = id;
  try {
    if (type === "image") {
      tab.imageUrl = convertFileSrc(path);
    } else {
      const info = await invoke<BinaryFileContent>("session_fs_read_bytes", {
        root,
        path,
      });
      tab.pdfData = base64ToBytes(info.content);
    }
  } catch (e) {
    tab.error = String(e);
  } finally {
    tab.loading = false;
  }
}

/** 释放标签后端资源：终端进程结束（幂等，失败静默） */
function disposeTab(tab: EditorTab): void {
  if (tab.kind === "terminal") {
    void invoke("terminal_kill", { id: tab.id }).catch(() => {});
  }
}

/**
 * 关闭标签：脏文件先挂起确认（pendingCloseId），确认后由 saveTabAndClose/
 * discardTabAndClose 完成；终端直接结束进程并移除，不做确认。
 */
export function closeTab(id: string): void {
  if (id === "chat") return;
  const tab = tabs.find((t) => t.id === id);
  if (!tab) return;
  if (tab.kind === "file" && tab.dirty) {
    pendingCloseId.value = id;
    return;
  }
  disposeTab(tab);
  removeTab(id);
}

export function cancelClose(): void {
  pendingCloseId.value = null;
}

export async function saveTabAndClose(id: string): Promise<void> {
  pendingCloseId.value = null;
  const ok = await saveFileTab(id);
  if (ok) removeTab(id);
}

export function discardTabAndClose(id: string): void {
  pendingCloseId.value = null;
  removeTab(id);
}

/**
 * 按谓词批量关闭标签：脏文件跳过计数、终端结束进程、其余直接移除。
 * 谓词基于遍历时的快照索引判定；快照遍历 + 按 id 移除，删除过程安全。
 */
function closeTabsMatching(
  pred: (tab: EditorTab, idx: number) => boolean,
): number {
  let skipped = 0;
  for (const [idx, tab] of [...tabs].entries()) {
    if (!pred(tab, idx)) continue;
    if (tab.kind === "file" && tab.dirty) {
      skipped++;
      continue;
    }
    disposeTab(tab);
    removeTab(tab.id);
  }
  return skipped;
}

/** 关闭其它所有标签（保留会话主标签）；返回跳过的未保存标签数量 */
export function closeAllOtherTabs(): number {
  return closeTabsMatching((tab) => tab.id !== "chat");
}

/** 关闭目标标签左侧所有可关闭标签（不含会话与目标本身）；返回跳过的未保存标签数量 */
export function closeTabsToLeft(id: string): number {
  const idx = tabs.findIndex((t) => t.id === id);
  if (idx < 0) return 0;
  return closeTabsMatching((tab, i) => i < idx && tab.id !== "chat");
}

/** 关闭目标标签右侧所有可关闭标签（不含会话与目标本身）；返回跳过的未保存标签数量 */
export function closeTabsToRight(id: string): number {
  const idx = tabs.findIndex((t) => t.id === id);
  if (idx < 0) return 0;
  return closeTabsMatching((tab, i) => i > idx && tab.id !== "chat");
}

function removeTab(id: string): void {
  const idx = tabs.findIndex((t) => t.id === id);
  if (idx < 0) return;
  const kind = tabs[idx].kind;
  const wasActive = activeTabId.value === id;
  tabs.splice(idx, 1);
  if (kind === "terminal") releaseTerminal(id);
  if (wasActive) {
    const next = tabs[Math.max(0, idx - 1)] ?? tabs[0];
    activeTabId.value = next ? next.id : "chat";
  }
}

/** 有未保存更改的文件标签（供关闭应用守卫使用） */
export function dirtyFileTabs(): FileEditorTab[] {
  return tabs.filter(
    (t): t is FileEditorTab => t.kind === "file" && t.dirty,
  );
}

/** 保存全部脏标签；全部成功返回 true（任一失败则不关闭应用） */
export async function saveAllDirtyTabs(): Promise<boolean> {
  const results = await Promise.all(dirtyFileTabs().map((t) => saveFileTab(t.id)));
  return results.every(Boolean);
}

/** 测试专用：清空标签状态并重建“对话”主标签 */
export function __resetEditorTabsForTest(): void {
  tabs.splice(0, tabs.length);
  activeTabId.value = "chat";
  pendingCloseId.value = null;
  initChatTab();
}
