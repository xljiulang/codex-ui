/** 外部文件变更后自动刷新标签：事件时刷新活动标签并标记非活动标签，
 *  非活动标签切回活动时补刷（保留视图状态，事件刷新在交互空闲时执行） */

import { invoke } from "@tauri-apps/api/core";
import { markRaw, watch } from "vue";
import { activeTabId, tabs, type Tab } from "../useTabs";
import { assetUrl } from "../../lib/asset";
import { docxToHtml } from "../../lib/docx";
import { detectEol, normalizeForEditor, stripBom } from "../../lib/editorFile";
import { formatTimeHMS, relPathOf } from "../../lib/format";
import { normalizeFsPath } from "../../lib/path";
import { TabKind } from "../../lib/tabs";
import { buildFileEditorState } from "./open";
import type {
  DocxEditorTab,
  FileEditorTab,
  PreviewEditorTab,
} from "./types";

/** Rust session_fs 事件载荷（与 session_fs.rs emit 结构一致） */
export interface FsChangedPayload {
  root: string;
  paths: string[];
}

/** 会话文件读取结果（与 Rust session_fs_read 返回结构一致） */
interface TextFileContent {
  content: string;
  validUtf8: boolean;
  byteSize: number;
}

/** 编辑区交互空闲阈值（鼠标/键盘最后一次交互后等待时长） */
const IDLE_MS = 1000;

let refreshing = false;
let lastInteractionAt = 0;
let deferredTimer: ReturnType<typeof setTimeout> | null = null;

const EDITOR_HOST_SELECTOR = ".text-editor-host, .docx-editor-host";

function editorHostOf(target: EventTarget | null): Element | null {
  return target instanceof Element ? target.closest(EDITOR_HOST_SELECTOR) : null;
}

function onPointerDown(e: PointerEvent) {
  if (editorHostOf(e.target)) lastInteractionAt = Date.now();
}

function onKeyDown(e: KeyboardEvent) {
  if (editorHostOf(e.target)) lastInteractionAt = Date.now();
}

if (typeof window !== "undefined") {
  window.addEventListener("pointerdown", onPointerDown, true);
  window.addEventListener("keydown", onKeyDown, true);
}

/** 工作区根归一化比较（Windows 忽略大小写与结尾分隔符） */
function sameRoot(a: string, b: string): boolean {
  return normalizeFsPath(a).toLowerCase() === normalizeFsPath(b).toLowerCase();
}

/** 相对路径归一化比较（事件路径为正斜杠，标签路径可能为反斜杠） */
function sameRelPath(a: string, b: string): boolean {
  const norm = (p: string) =>
    p.replace(/\\/g, "/").replace(/^\.\//, "").toLowerCase();
  return norm(a) === norm(b);
}

/** 可自动刷新的标签类型（文本 / .docx / 预览） */
function isFileLikeTab(
  tab: Tab | undefined,
): tab is FileEditorTab | DocxEditorTab | PreviewEditorTab {
  return (
    !!tab &&
    (tab.kind === TabKind.File ||
      tab.kind === TabKind.Docx ||
      tab.kind === TabKind.Preview)
  );
}

/** 标签是否命中事件载荷（工作区 + 相对路径；payload 缺失视为命中） */
function matchesPayload(
  tab: FileEditorTab | DocxEditorTab | PreviewEditorTab,
  payload?: FsChangedPayload,
): boolean {
  if (payload?.root && !sameRoot(tab.workspace, payload.root)) return false;
  if (
    payload?.paths?.length &&
    // 事件 paths 为相对工作区的路径，标签 path 可能是绝对路径：统一转相对再比
    !payload.paths.some((p) =>
      sameRelPath(relPathOf(tab.workspace, tab.path), p),
    )
  ) {
    return false;
  }
  return true;
}

/**
 * 事件驱动的活动标签自动刷新：
 * - 先标记所有命中标签为 stale（非活动标签切回活动时补刷）；
 * - 活动标签命中时立即刷新：脏/保存中/加载中跳过，交互中延迟到空闲后单次重试；
 * - 文本/富文本刷新保留滚动与光标，PDF 由组件保持页/缩放。
 */
export async function refreshTabsFromFs(
  payload?: FsChangedPayload,
): Promise<void> {
  for (const t of [...tabs]) {
    if (!isFileLikeTab(t)) continue;
    if (t.missing) continue;
    if (t.loading) continue;
    if (t.kind !== TabKind.Preview && t.saving) continue;
    if (!matchesPayload(t, payload)) continue;
    t.stale = true;
  }
  if (refreshing) return;
  const tab = tabs.find((t) => t.id === activeTabId.value);
  if (!isFileLikeTab(tab) || tab.missing || !matchesPayload(tab, payload)) return;
  if (tab.kind === TabKind.File || tab.kind === TabKind.Docx) {
    const editable = tab as FileEditorTab | DocxEditorTab;
    if (editable.dirty) {
      editable.status = "文件已在外部变更（存在未保存修改）";
      return;
    }
  }
  const idleWait = Math.max(0, lastInteractionAt + IDLE_MS - Date.now());
  if (idleWait > 0) {
    if (deferredTimer) clearTimeout(deferredTimer);
    deferredTimer = setTimeout(() => {
      deferredTimer = null;
      void refreshTabsFromFs(payload);
    }, idleWait);
    return;
  }

  await refreshTab(tab);
}

/**
 * 非活动标签切回活动时的补刷：watcher 曾标记过 stale 才执行（零额外开销），
 * 脏标签跳过（不覆盖未保存修改）并清除标记；预览无脏状态直接重载。
 */
watch(activeTabId, (id) => {
  const tab = tabs.find((t) => t.id === id);
  if (!isFileLikeTab(tab) || !tab.stale) return;
  if (tab.missing) {
    tab.stale = false;
    return;
  }
  if (tab.loading) return;
  if (tab.kind !== TabKind.Preview && tab.saving) return;
  if (tab.kind === TabKind.File || tab.kind === TabKind.Docx) {
    const editable = tab as FileEditorTab | DocxEditorTab;
    if (editable.dirty) {
      tab.stale = false;
      return;
    }
  }
  tab.stale = false;
  void refreshTab(tab);
});

/** 单标签刷新核心（事件与激活补刷共用）；结束后清除 stale */
async function refreshTab(
  tab: FileEditorTab | DocxEditorTab | PreviewEditorTab,
): Promise<void> {
  if (refreshing) return;
  refreshing = true;
  try {
    if (tab.kind === TabKind.File) {
      await refreshFileTab(tab);
    } else if (tab.kind === TabKind.Docx) {
      await refreshDocxTab(tab);
    } else {
      await refreshPreviewTab(tab);
    }
  } finally {
    refreshing = false;
    tab.stale = false;
  }
}

/** 文本标签：内容未变跳过；变化则重建状态并保留光标/滚动位置 */
async function refreshFileTab(tab: FileEditorTab): Promise<void> {
  try {
    const info = await invoke<TextFileContent>("session_fs_read", {
      workspace: tab.workspace,
      path: tab.path,
    });
    const { text, hadBom } = stripBom(info.content);
    const eol = detectEol(text);
    const doc = normalizeForEditor(text, eol);
    if (tab.editorState && doc === tab.editorState.doc.toString()) return;
    const state = await buildFileEditorState(
      tab,
      doc,
      info.validUtf8,
      tab.cursor,
    );
    tab.readOnly = !info.validUtf8;
    tab.eol = eol;
    tab.hadBom = hadBom;
    tab.byteSize = info.byteSize;
    tab.editorState = markRaw(state);
    tab.savedText = markRaw(state.doc);
    tab.dirty = false;
    tab.status = `已从磁盘刷新 ${formatTimeHMS(Date.now())}`;
  } catch {
    tab.missing = true;
    tab.status = "外部刷新失败：文件可能已被删除";
  }
}

/** .docx 标签：重新导入并替换编辑器内容，恢复选区与宿主滚动位置 */
async function refreshDocxTab(tab: DocxEditorTab): Promise<void> {
  try {
    const buf = await invoke<ArrayBuffer>("session_fs_read_bytes", {
      workspace: tab.workspace,
      path: tab.path,
    });
    const { html } = await docxToHtml(buf);
    if (html === tab.initialHtml) return;
    const editor = tab.editor;
    if (editor) {
      const host =
        editor.view.dom instanceof Element
          ? (editor.view.dom.closest(".docx-editor-host") as HTMLElement | null)
          : null;
      const scrollTop = host?.scrollTop ?? 0;
      const from = editor.state.selection.from;
      editor.commands.setContent(html);
      const size = editor.state.doc.content.size;
      if (from <= size) {
        editor.commands.setTextSelection(from);
      }
      if (host) {
        host.scrollTop = scrollTop;
        // ProseMirror 恢复选区后可能再滚动到光标，下一帧兜底恢复宿主滚动
        requestAnimationFrame(() => {
          host.scrollTop = scrollTop;
        });
      }
      tab.dirty = false;
    }
    tab.initialHtml = html;
    tab.byteSize = buf.byteLength;
    tab.status = `已从磁盘刷新 ${formatTimeHMS(Date.now())}`;
  } catch {
    tab.missing = true;
    tab.status = "外部刷新失败：文件可能已被删除";
  }
}

/** 预览标签：图片击穿缓存重取，PDF / XLSX 替换字节由组件重载（保持页/表） */
async function refreshPreviewTab(tab: PreviewEditorTab): Promise<void> {
  try {
    if (tab.previewType === "image") {
      tab.imageUrl = `${assetUrl(tab.path)}?t=${Date.now()}`;
    } else {
      const bytes = new Uint8Array(
        await invoke<ArrayBuffer>("session_fs_read_bytes", {
          workspace: tab.workspace,
          path: tab.path,
        }),
      );
      if (tab.previewType === "pdf") {
        tab.pdfData = bytes;
      } else {
        tab.xlsxData = bytes;
      }
    }
  } catch {
    tab.missing = true;
    // 预览标签无状态栏：刷新失败静默（图片/PDF/XLSX 组件各自展示错误态）
  }
}

/** 仅测试用：重置模块状态（含未完成的延迟刷新） */
export function __resetRefreshForTest(): void {
  refreshing = false;
  lastInteractionAt = 0;
  if (deferredTimer) {
    clearTimeout(deferredTimer);
    deferredTimer = null;
  }
}

/** 仅测试用：模拟编辑区最近交互时间，验证延迟刷新 */
export function __setRefreshInteractionForTest(at: number): void {
  lastInteractionAt = at;
}
