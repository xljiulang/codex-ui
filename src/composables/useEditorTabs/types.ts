import type { Compartment, EditorState, Text } from "@codemirror/state";
import type { EditorEol } from "../../lib/editorFile";
import type { DiffPreviewKind, GitCommitDetail } from "../../lib/gitChanges";
import type { PreviewType } from "../../lib/preview";
import type { DiffRow } from "../../lib/types";
import { TabIcon, TabKind, type EditorTabBase } from "../../lib/tabs";

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
  /** 编辑区垂直滚动位置（TextEditorPane 写回，外部刷新后恢复） */
  scrollTop: number;
  /** 外部变更已发生但尚未刷新（非活动期间被 watcher 标记，切回活动时补刷） */
  stale: boolean;
  /** 文件已从磁盘丢失（外部删除等）：后续自动刷新跳过，避免反复读不存在的文件 */
  missing?: boolean;
  status: string;
  /** CodeMirror 状态（非响应式，避免深度代理开销）；切换标签时由编辑组件 setState */
  editorState: EditorState | null;
  savedText: Text | null;
  wrapCompartment: Compartment | null;
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
  /** 预览类型：pdf → pdf.js 渲染；image → asset URL 直显；xlsx → 表格预览 */
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
  /** XLSX 预览：后端读取的原始字节（SheetJS read 数据源，组件解析渲染） */
  xlsxData: Uint8Array | null;
  /** DOCX 预览：后端读取的原始字节（docx-preview renderAsync 数据源，组件渲染） */
  docxData: Uint8Array | null;
  /** PPTX 预览：后端读取的原始字节（pptx-preview preview 数据源，组件渲染） */
  pptxData: Uint8Array | null;
  /** XLSX 当前工作表序号：随标签持久，外部刷新后按新工作表数夹紧 */
  xlsxSheetIndex: number;
  /** PDF 页数：组件加载文档后回填 */
  pageCount: number | null;
  /** 外部变更已发生但尚未刷新（非活动期间被 watcher 标记，切回活动时补刷） */
  stale: boolean;
  /** 文件已从磁盘丢失（外部删除等）：后续自动刷新跳过，避免反复读不存在的文件 */
  missing?: boolean;
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

/** 设置标签：全应用唯一的设置页（统一列表恒在最后，可关闭） */
export interface SettingsTab extends EditorTabBase {
  kind: (typeof TabKind)["Settings"];
  id: string;
  title: string;
  icon: (typeof TabIcon)["Settings"];
  workspace: null;
  loading: false;
}

export type EditorTab =
  | FileEditorTab
  | DiffEditorTab
  | PreviewEditorTab
  | TerminalEditorTab
  | CommitEditorTab
  | SettingsTab;
