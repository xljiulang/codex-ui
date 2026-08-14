import { computed, markRaw, reactive, ref, shallowReactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Compartment, EditorState, Text } from "@codemirror/state";
import {
  buildSaveContent,
  detectEol,
  normalizeForEditor,
  stripBom,
  type EditorEol,
} from "../lib/editorFile";
import { pathBaseName } from "../lib/format";
import type { DiffRow } from "../lib/types";

/** 会话文件读取结果（与 Rust session_fs_read 返回结构一致） */
interface TextFileContent {
  content: string;
  validUtf8: boolean;
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
}

export type EditorTab = ChatEditorTab | FileEditorTab | DiffEditorTab;

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

function nowTime(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
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

/** 关闭标签：脏文件先挂起确认（pendingCloseId），确认后由 saveTabAndClose/discardTabAndClose 完成 */
export function closeTab(id: string): void {
  if (id === "chat") return;
  const tab = tabs.find((t) => t.id === id);
  if (!tab) return;
  if (tab.kind === "file" && tab.dirty) {
    pendingCloseId.value = id;
    return;
  }
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

function removeTab(id: string): void {
  const idx = tabs.findIndex((t) => t.id === id);
  if (idx < 0) return;
  const wasActive = activeTabId.value === id;
  tabs.splice(idx, 1);
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
