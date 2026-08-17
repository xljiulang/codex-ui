import { markRaw, reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { EditorState } from "@codemirror/state";
import {
  buildSaveContent,
  detectEol,
  normalizeForEditor,
  stripBom,
} from "../../lib/editorFile";
import { formatTimeHMS, pathBaseName } from "../../lib/format";
import { languageFromPath } from "../../lib/highlight";
import { assetUrl } from "../../lib/asset";
import type { DiffPreviewKind, GitCommitDetail } from "../../lib/gitChanges";
import type { PreviewType } from "../../lib/preview";
import { docxToHtml, jsonToDocx } from "../../lib/docx";
import type { DiffRow } from "../../lib/types";
import {
  attachTerminal,
  ensureTerminalListeners,
} from "../useTerminalEvents";
import { TabIcon, TabKind } from "../../lib/tabs";
import { activateTab, activeTabId, insertTab, tabs } from "../useTabs";
import { store } from "../useCodex/store";
import type {
  CommitEditorTab,
  DocxEditorTab,
  DiffEditorTab,
  DiffPreviewParams,
  EditorTab,
  FileEditorTab,
  PreviewEditorTab,
  SettingsTab,
  TerminalEditorTab,
} from "./types";

/** 会话文件读取结果（与 Rust session_fs_read 返回结构一致） */
interface TextFileContent {
  content: string;
  validUtf8: boolean;
  byteSize: number;
}

function fileTabId(workspace: string, path: string): string {
  return "file:" + JSON.stringify([workspace, path]);
}

function diffTabId(p: DiffPreviewParams): string {
  return "diff:" + JSON.stringify([p.workspace, p.path, p.kind]);
}

function previewTabId(
  type: PreviewType,
  workspace: string,
  path: string,
): string {
  return `preview:${type}:${JSON.stringify([workspace, path])}`;
}

/** 设置标签固定 id：全局唯一，头部设置按钮据此查找/激活/关闭 */
export const SETTINGS_TAB_ID = "settings";

/**
 * 打开设置标签：不存在则创建（统一列表恒在最后）并激活，已存在则直接激活。
 * 关闭入口复用统一 closeAnyTab（关闭活动标签自动回到相邻标签）。
 */
export function openSettingsTab(): void {
  const existing = tabs.find((t) => t.id === SETTINGS_TAB_ID);
  if (existing) {
    activateTab(SETTINGS_TAB_ID);
    return;
  }
  const tab: SettingsTab = {
    id: SETTINGS_TAB_ID,
    kind: TabKind.Settings,
    title: "设置",
    icon: TabIcon.Settings,
    workspace: null,
    loading: false,
  };
  insertTab(tab);
  activateTab(SETTINGS_TAB_ID);
}

/**
 * 按标签当前配置构建 CodeMirror 状态（首次打开与外部变更刷新共用）：
 * 语言/只读/换行/Compartment 均取自标签，回写 dirty/cursor/editorState 回调一致。
 * selection 可选：恢复光标位置（行/列 1 起，超出自动夹紧）。
 */
export async function buildFileEditorState(
  tab: FileEditorTab,
  doc: string,
  validUtf8: boolean,
  selection?: { line: number; col: number },
): Promise<EditorState> {
  const [{ buildEditorExtensions, createEditorState, languageForPath }, { Compartment }] =
    await Promise.all([
      import("../../lib/editorSetup"),
      import("@codemirror/state"),
    ]);
  const wrapCompartment = tab.wrapCompartment ?? new Compartment();
  if (!tab.wrapCompartment) {
    tab.wrapCompartment = markRaw(wrapCompartment);
  }
  return createEditorState(
    doc,
    buildEditorExtensions({
      language: languageForPath(tab.path),
      readOnly: !validUtf8,
      wrap: tab.wrap,
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
    selection,
  );
}

function commitTabId(workspace: string, hash: string): string {
  return "commit:" + JSON.stringify([workspace, hash]);
}

function commitFileDiffTabId(
  workspace: string,
  hash: string,
  path: string,
): string {
  return "diff:commit:" + JSON.stringify([workspace, hash, path]);
}

/** 终端标签自增序号：保证同一毫秒内连续多开也生成不同 id */
let terminalSeq = 0;

/**
 * 打开提交详情标签：同一提交已打开则直接激活；否则新建标签并异步拉取
 * git_changes_commit_detail 填充 detail，失败保留标签并记录错误。
 */
export async function openCommitTab(
  workspace: string,
  hash: string,
  subject: string,
): Promise<void> {
  const id = commitTabId(workspace, hash);
  if (tabs.some((t) => t.id === id)) {
    activeTabId.value = id;
    return;
  }
  const tab = reactive({
    kind: TabKind.Commit,
    id,
    workspace,
    hash,
    title: subject || hash.slice(0, 7) || "提交",
    icon: TabIcon.Commit,
    loading: true,
    error: "",
    detail: null,
  }) as unknown as CommitEditorTab;
  insertTab(tab);
  activeTabId.value = id;
  try {
    const detail = await invoke<GitCommitDetail>("git_changes_commit_detail", {
      workspace,
      hash,
    });
    tab.detail = detail ?? null;
  } catch (e) {
    tab.error = String(e);
  } finally {
    tab.loading = false;
  }
}

/**
 * 打开提交内单个文件的 diff 标签：复用普通 Diff 标签渲染，id 含 hash 避免与
 * 工作区 diff 冲突；rows 由 git_changes_commit_file_diff 直接返回。
 */
export async function openCommitFileDiffTab(
  workspace: string,
  hash: string,
  path: string,
  changeKind: DiffPreviewKind,
): Promise<void> {
  const id = commitFileDiffTabId(workspace, hash, path);
  if (tabs.some((t) => t.id === id)) {
    activeTabId.value = id;
    return;
  }
  const tab = reactive({
    kind: TabKind.Diff,
    id,
    path,
    changeKind,
    workspace,
    title: pathBaseName(path) || path,
    icon: TabIcon.File,
    loading: true,
    error: "",
    rows: [],
    fallback: "",
    brief: false,
  }) as unknown as DiffEditorTab;
  insertTab(tab);
  activeTabId.value = id;
  try {
    const rows = await invoke<DiffRow[]>("git_changes_commit_file_diff", {
      workspace,
      hash,
      path,
    });
    tab.rows = rows ?? [];
  } catch (e) {
    tab.error = String(e);
  } finally {
    tab.loading = false;
  }
}

/**
 * 打开终端标签：每次调用都新建独立会话（支持同目录多开），生成唯一 id、
 * 激活标签后向后端发起 terminal_spawn；失败保留标签并记录错误。
 */
export async function openTerminalTab(workspace: string): Promise<void> {
  const id = `terminal:${++terminalSeq}:${Date.now()}`;
  const tab = reactive({
    kind: TabKind.Terminal,
    id,
    workspace,
    // 终端标签标题跟随所选 Shell（与后端 spawn 读取同一份设置），多开时同名；
    // 格式为「终端 (cmd) / 终端 (PowerShell)」，用户可右键「重命名」覆盖
    title:
      store.settings.terminal_shell === "powershell"
        ? "终端 (PowerShell)"
        : "终端 (cmd)",
    icon: TabIcon.Terminal,
    loading: true,
    error: "",
    busy: false,
    exited: false,
    exitCode: null,
  }) as unknown as TerminalEditorTab;
  insertTab(tab);
  activeTabId.value = id;
  try {
    // 先注册全局事件监听并建立缓冲，再 spawn，避免启动输出（含 ConPTY DSR
    // 查询）在懒加载面板挂载前丢失导致首个终端空白。
    await ensureTerminalListeners();
    attachTerminal(id);
    await invoke("terminal_spawn", { id, workspace });
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

/**
 * 打开文本文件标签：已打开则直接激活；否则新建标签并异步读取内容、
 * 构建 CodeMirror 状态（按标签闭包维护 dirty/cursor/最新 state）。
 */
export async function openFileTab(
  workspace: string,
  path: string,
): Promise<void> {
  const id = fileTabId(workspace, path);
  if (tabs.some((t) => t.id === id)) {
    activeTabId.value = id;
    return;
  }
  // reactive 的类型会深展开 EditorState，这里用断言还原为标签类型；
  // 运行时编辑器状态在赋值时均已 markRaw，不会被深度代理。
  const tab = reactive({
    kind: TabKind.File,
    id,
    workspace,
    path,
    title: pathBaseName(path) || path,
    icon: TabIcon.File,
    loading: true,
    error: "",
    readOnly: false,
    dirty: false,
    saving: false,
    wrap: false,
    // Markdown 文件默认渲染模式（与 TextEditorPane.isMarkdown 同一判断，覆盖 .md/.markdown）
    markdownPreview: languageFromPath(path) === "markdown",
    eol: "\n",
    hadBom: false,
    byteSize: null,
    cursor: { line: 1, col: 1 },
    scrollTop: 0,
    stale: false,
    status: "",
    editorState: null,
    savedText: null,
    wrapCompartment: null,
  }) as unknown as FileEditorTab;
  insertTab(tab);
  activeTabId.value = id;
  try {
    const info = await invoke<TextFileContent>("session_fs_read", {
      workspace,
      path,
    });
    const ro = !info.validUtf8;
    const { text, hadBom } = stripBom(info.content);
    const lineEol = detectEol(text);
    const doc = normalizeForEditor(text, lineEol);
    const state = await buildFileEditorState(tab, doc, info.validUtf8);
    tab.readOnly = ro;
    tab.eol = lineEol;
    tab.hadBom = hadBom;
    tab.byteSize = info.byteSize;
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
    (t): t is FileEditorTab => t.kind === TabKind.File && t.id === id,
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
      workspace: tab.workspace,
      path: tab.path,
      content,
    });
    tab.savedText = markRaw(tab.editorState.doc);
    tab.dirty = false;
    tab.status = `已保存 ${formatTimeHMS(Date.now())}`;
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
    kind: TabKind.Diff,
    id,
    path: params.path,
    changeKind: params.kind,
    workspace: params.workspace,
    title: pathBaseName(params.path) || params.path,
    icon: TabIcon.File,
    loading: true,
    error: "",
    rows: [],
    fallback: params.diff,
    brief: false,
  }) as unknown as DiffEditorTab;
  insertTab(tab);
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
 * 打开特殊文件预览标签（PDF / 图像 / XLSX）：已打开则激活；否则新建标签并异步准备数据。
 * 图像直接经 asset 协议取 URL；PDF / XLSX 经 session_fs_read_bytes 直取原始字节（ArrayBuffer）。
 */
export async function openPreviewTab(
  type: PreviewType,
  workspace: string,
  path: string,
): Promise<void> {
  const id = previewTabId(type, workspace, path);
  if (tabs.some((t) => t.id === id)) {
    activeTabId.value = id;
    return;
  }
  const tab = reactive({
    kind: TabKind.Preview,
    previewType: type,
    id,
    workspace,
    path,
    title: pathBaseName(path) || path,
    icon: TabIcon.File,
    loading: true,
    error: "",
    imageUrl: "",
    pdfData: null,
    xlsxData: null,
    xlsxSheetIndex: 0,
    pageCount: null,
    stale: false,
  }) as unknown as PreviewEditorTab;
  insertTab(tab);
  activeTabId.value = id;
  try {
    if (type === "image") {
      tab.imageUrl = assetUrl(path);
    } else {
      const bytes = new Uint8Array(
        await invoke<ArrayBuffer>("session_fs_read_bytes", {
          workspace,
          path,
        }),
      );
      if (type === "pdf") {
        tab.pdfData = bytes;
      } else {
        tab.xlsxData = bytes;
      }
    }
  } catch (e) {
    tab.error = String(e);
  } finally {
    tab.loading = false;
  }
}

/**
 * 打开 .docx 富文本编辑标签：已打开则直接激活；否则新建标签并异步读取
 * 二进制字节 → mammoth 转 HTML 存入标签，由 DocxEditorPane 创建 TipTap 实例。
 */
export async function openDocxTab(
  workspace: string,
  path: string,
): Promise<void> {
  const id = fileTabId(workspace, path);
  if (tabs.some((t) => t.id === id)) {
    activeTabId.value = id;
    return;
  }
  const tab = reactive({
    kind: TabKind.Docx,
    id,
    workspace,
    path,
    title: pathBaseName(path) || path,
    icon: TabIcon.File,
    loading: true,
    error: "",
    dirty: false,
    saving: false,
    status: "",
    byteSize: null,
    stale: false,
    initialHtml: null,
    editor: null,
  }) as unknown as DocxEditorTab;
  insertTab(tab);
  activeTabId.value = id;
  try {
    const buf = await invoke<ArrayBuffer>("session_fs_read_bytes", {
      workspace,
      path,
    });
    const { html, warnings } = await docxToHtml(buf);
    tab.initialHtml = html;
    tab.byteSize = buf.byteLength;
    if (warnings.length > 0) {
      tab.status = `导入提示：${warnings[0]}`;
    }
  } catch (e) {
    tab.error = String(e);
  } finally {
    tab.loading = false;
  }
}

/** 保存 .docx 标签：编辑器 JSON → docx 字节 → 直接覆盖原文件 */
export async function saveDocxTab(id: string): Promise<boolean> {
  const tab = tabs.find(
    (t): t is DocxEditorTab => t.kind === TabKind.Docx && t.id === id,
  );
  if (!tab || !tab.editor || !tab.dirty || tab.saving) {
    return false;
  }
  tab.saving = true;
  try {
    const { base64, warnings } = await jsonToDocx(tab.editor.getJSON());
    await invoke("session_fs_write_bytes", {
      workspace: tab.workspace,
      path: tab.path,
      content: base64,
    });
    tab.dirty = false;
    tab.status = `已保存 ${formatTimeHMS(Date.now())}${
      warnings.length > 0 ? `（${warnings[0]}）` : ""
    }`;
    return true;
  } catch (e) {
    tab.status = `保存失败：${String(e)}`;
    return false;
  } finally {
    tab.saving = false;
  }
}

/** 运行中终端判定：命令执行中（busy）且未退出、无错误（与标签呼吸灯同源） */
export function isTerminalBusy(tab: EditorTab): boolean {
  return tab.kind === TabKind.Terminal && tab.busy && !tab.exited && !tab.error;
}

/** 文件是否已打开（存在 workspace+path 相同的文件编辑器或预览标签；diff 不计） */
export function isFileTabOpen(workspace: string, path: string): boolean {
  return tabs.some(
    (t) =>
      (t.kind === TabKind.File ||
        t.kind === TabKind.Preview ||
        t.kind === TabKind.Docx) &&
      t.workspace === workspace &&
      t.path === path,
  );
}
