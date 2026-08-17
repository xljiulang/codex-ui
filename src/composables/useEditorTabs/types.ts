import type { Compartment, EditorState, Text } from "@codemirror/state";
import type { EditorEol } from "../../lib/editorFile";
import type { DiffPreviewKind, GitCommitDetail } from "../../lib/gitChanges";
import type { PreviewType } from "../../lib/preview";
import type { DiffRow } from "../../lib/types";
import type { Editor } from "@tiptap/vue-3";
import { TabKind, type EditorTabBase } from "../../lib/tabs";

/** diff 预览参数（与 Rust DiffPreviewParams 结构一致） */
export interface DiffPreviewParams {
  path: string;
  kind: DiffPreviewKind;
  diff: string;
  workspace: string;
}

export interface FileEditorTab extends EditorTabBase {
  kind: (typeof TabKind)["File"];
  id: string;
  workspace: string;
  path: string;
  title: string;
  loading: boolean;
  error: string;
  readOnly: boolean;
  dirty: boolean;
  saving: boolean;
  wrap: boolean;
  /** Markdown 预览/编辑切换态：随标签持久化，切换标签后保持切出时状态 */
  markdownPreview: boolean;
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

/** .docx 富文本编辑标签：TipTap 编辑器实例随标签持久（非响应式） */
export interface DocxEditorTab extends EditorTabBase {
  kind: (typeof TabKind)["Docx"];
  id: string;
  workspace: string;
  path: string;
  title: string;
  loading: boolean;
  error: string;
  dirty: boolean;
  saving: boolean;
  status: string;
  byteSize: number | null;
  /** 首次导入的 HTML（TipTap 数据源），markRaw 存储避免响应式代理 */
  initialHtml: string | null;
  /** TipTap 编辑器实例（markRaw），由 DocxEditorPane 创建后回填 */
  editor: Editor | null;
}

export interface DiffEditorTab extends EditorTabBase {
  kind: (typeof TabKind)["Diff"];
  id: string;
  path: string;
  /** diff 变化类型：add / delete / modify */
  changeKind: DiffPreviewKind;
  workspace: string;
  title: string;
  loading: boolean;
  error: string;
  rows: DiffRow[];
  /** 原始 unified diff，行解析失败时回退展示 */
  fallback: string;
  /** 简要模式：仅显示变更行（隐藏未变化上下文） */
  brief: boolean;
}

export interface PreviewEditorTab extends EditorTabBase {
  kind: (typeof TabKind)["Preview"];
  /** 预览类型：pdf → pdf.js 渲染；image → asset URL 直显 */
  previewType: PreviewType;
  id: string;
  workspace: string;
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

export interface TerminalEditorTab extends EditorTabBase {
  kind: (typeof TabKind)["Terminal"];
  id: string;
  /** 终端启动目录（绝对路径） */
  workspace: string;
  title: string;
  loading: boolean;
  error: string;
  /** 命令执行中：回车/粘贴换行置位，收到提示符标记（OSC 133;D）后熄灭 */
  busy: boolean;
  /** 进程已退出（收到 terminal/exit 事件后置位） */
  exited: boolean;
  exitCode: number | null;
}

export interface CommitEditorTab extends EditorTabBase {
  kind: (typeof TabKind)["Commit"];
  id: string;
  /** 仓库根目录（绝对路径） */
  workspace: string;
  /** 完整提交哈希 */
  hash: string;
  title: string;
  loading: boolean;
  error: string;
  /** 提交详情（git_changes_commit_detail 返回；加载完成后填充） */
  detail: GitCommitDetail | null;
}

export type EditorTab =
  | FileEditorTab
  | DocxEditorTab
  | DiffEditorTab
  | PreviewEditorTab
  | TerminalEditorTab
  | CommitEditorTab;
