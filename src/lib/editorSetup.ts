import {
  Compartment,
  EditorSelection,
  EditorState,
  type Extension,
  type SelectionRange,
  type Text,
} from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from "@codemirror/commands";
import { search, searchKeymap } from "@codemirror/search";
import {
  HighlightStyle,
  LanguageSupport,
  StreamLanguage,
  indentUnit,
  syntaxHighlighting,
} from "@codemirror/language";
import { tags } from "@lezer/highlight";

import { css } from "@codemirror/lang-css";
import { cpp } from "@codemirror/lang-cpp";
import { go } from "@codemirror/lang-go";
import { html } from "@codemirror/lang-html";
import { java } from "@codemirror/lang-java";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { php } from "@codemirror/lang-php";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { sql } from "@codemirror/lang-sql";
import { xml } from "@codemirror/lang-xml";
import { yaml } from "@codemirror/lang-yaml";

import {
  csharp,
  kotlin,
} from "@codemirror/legacy-modes/mode/clike";
import { diff } from "@codemirror/legacy-modes/mode/diff";
import { powerShell } from "@codemirror/legacy-modes/mode/powershell";
import { properties } from "@codemirror/legacy-modes/mode/properties";
import { ruby } from "@codemirror/legacy-modes/mode/ruby";
import { shell } from "@codemirror/legacy-modes/mode/shell";

/** 语法高亮配色：与 diff/聊天区 hljs 调色板共用 --syntax-* 令牌，随主题切换 */
export const editorHighlightStyle = HighlightStyle.define([
  { tag: tags.comment, color: "var(--syntax-comment)" },
  {
    tag: [tags.keyword, tags.modifier, tags.controlKeyword, tags.operatorKeyword],
    color: "var(--syntax-keyword)",
  },
  {
    tag: [tags.string, tags.special(tags.string), tags.regexp, tags.attributeValue],
    color: "var(--syntax-string)",
  },
  {
    tag: [tags.number, tags.integer, tags.float, tags.bool, tags.null, tags.meta],
    color: "var(--syntax-number)",
  },
  {
    tag: [tags.function(tags.variableName), tags.function(tags.propertyName), tags.labelName],
    color: "var(--syntax-function)",
  },
  {
    tag: [
      tags.typeName,
      tags.className,
      tags.namespace,
      tags.propertyName,
      tags.attributeName,
      tags.variableName,
    ],
    color: "var(--syntax-type)",
  },
  {
    tag: [tags.standard(tags.variableName)],
    color: "var(--syntax-const)",
  },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.strong, fontWeight: "600" },
]);

/** 编辑器外观主题 spec：CSS 变量驱动，深色终端质感（与旧预览一致）。
 * 独立命名便于测试（如查找面板按钮配色）。 */
export const editorThemeSpec = {
  "&": {
    height: "100%",
    fontSize: "12px",
    backgroundColor: "var(--console-bg-deep)",
    color: "var(--console-text)",
  },
  ".cm-scroller": {
    fontFamily: "var(--mono)",
    lineHeight: "1.55",
    overflow: "auto",
  },
  ".cm-content": {
    caretColor: "var(--accent)",
    padding: "10px 0",
  },
  ".cm-line": {
    padding: "0 12px 0 8px",
  },
  ".cm-gutters": {
    backgroundColor: "transparent",
    color: "var(--text-faint)",
    border: "none",
    paddingLeft: "8px",
  },
  ".cm-activeLine": {
    backgroundColor: "rgba(var(--accent-rgb), 0.07)",
  },
  ".cm-activeLineGutter": {
    backgroundColor: "rgba(var(--accent-rgb), 0.09)",
    color: "var(--text-dim)",
  },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "rgba(var(--accent-rgb), 0.28)",
  },
  ".cm-cursor, .cm-dropCursor": {
    borderLeftColor: "var(--accent)",
  },
  ".cm-panels": {
    backgroundColor: "var(--bg-panel)",
    color: "var(--text)",
  },
  ".cm-panels.cm-panels-top": {
    borderBottom: "1px solid var(--border)",
  },
  ".cm-panels .cm-textfield": {
    backgroundColor: "var(--bg-input)",
    color: "var(--text)",
    border: "1px solid var(--border)",
    borderRadius: "6px",
    fontFamily: "var(--mono)",
    fontSize: "12px",
  },
  ".cm-panels .cm-button": {
    backgroundColor: "var(--bg-active)",
    color: "var(--text-bright)",
    // 覆盖 CodeMirror 基础主题 &dark .cm-button 的近黑渐变，避免浅色主题下黑字黑底
    backgroundImage: "none",
    border: "1px solid var(--border)",
    borderRadius: "6px",
    cursor: "pointer",
    fontSize: "12px",
  },
  ".cm-panels label": {
    color: "var(--text-dim)",
    fontSize: "12px",
  },
  ".cm-searchMatch": {
    backgroundColor: "rgba(var(--accent-rgb), 0.32)",
    outline: "none",
  },
  ".cm-searchMatch.cm-searchMatch-selected": {
    backgroundColor: "rgba(229, 214, 160, 0.45)",
  },
};

/** 编辑器外观主题（深色基调；控件颜色由 CSS 变量随应用主题切换） */
export const editorTheme = EditorView.theme(editorThemeSpec, { dark: true });

/** 扩展名 → CodeMirror 语言扩展；无法识别返回 null（纯文本） */
export function languageForPath(path: string): Extension | null {
  const name = path.split(/[\\/]/).pop() ?? "";
  const dot = name.lastIndexOf(".");
  const ext = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
  switch (ext) {
    case "ts":
    case "mts":
    case "cts":
    case "tsx":
      return javascript({ typescript: true, jsx: ext === "tsx" });
    case "js":
    case "jsx":
    case "mjs":
    case "cjs":
      return javascript({ jsx: ext === "jsx" });
    case "py":
    case "pyw":
      return python();
    case "rs":
      return rust();
    case "cs":
      return new LanguageSupport(StreamLanguage.define(csharp));
    case "c":
    case "h":
    case "cpp":
    case "cc":
    case "cxx":
    case "hpp":
    case "hh":
    case "hxx":
      return cpp();
    case "go":
      return go();
    case "java":
      return java();
    case "kt":
    case "kts":
      return new LanguageSupport(StreamLanguage.define(kotlin));
    case "json":
      return json();
    case "md":
    case "markdown":
      return markdown();
    case "yml":
    case "yaml":
      return yaml();
    case "sh":
    case "bash":
    case "zsh":
      return new LanguageSupport(StreamLanguage.define(shell));
    case "ps1":
    case "psm1":
      return new LanguageSupport(StreamLanguage.define(powerShell));
    case "sql":
      return sql();
    case "css":
      return css();
    case "html":
    case "htm":
      return html();
    case "xml":
    case "svg":
      return xml();
    case "ini":
    case "cfg":
      return new LanguageSupport(StreamLanguage.define(properties));
    case "php":
      return php();
    case "rb":
      return new LanguageSupport(StreamLanguage.define(ruby));
    case "diff":
    case "patch":
      return new LanguageSupport(StreamLanguage.define(diff));
    default:
      return null;
  }
}

export interface EditorExtensionsOptions {
  language: Extension | null;
  readOnly: boolean;
  wrap: boolean;
  wrapCompartment: Compartment;
  /** 返回当前“已保存”文档（Text 不可变，保存成功后被替换） */
  savedText: () => Text | null;
  onDirtyChange: (dirty: boolean) => void;
  onCursorChange: (line: number, col: number) => void;
  onSave: () => void;
  /** 每次文档/选区更新后回调最新 EditorState（标签页用于回写保存状态，无需 DOM 视图） */
  onStateChange?: (state: EditorState) => void;
}

/** 组装编辑器扩展：行号、活动行、历史、查找/替换、快捷键、主题、高亮、脏状态监听 */
export function buildEditorExtensions(
  opts: EditorExtensionsOptions,
): Extension[] {
  return [
    lineNumbers(),
    highlightActiveLineGutter(),
    highlightActiveLine(),
    drawSelection(),
    history(),
    EditorState.readOnly.of(opts.readOnly),
    syntaxHighlighting(editorHighlightStyle),
    editorTheme,
    indentUnit.of("    "),
    ...(opts.language ? [opts.language] : []),
    opts.wrapCompartment.of(opts.wrap ? EditorView.lineWrapping : []),
    search({ top: true }),
    EditorView.contentAttributes.of({
      spellcheck: "false",
      autocapitalize: "off",
      autocomplete: "off",
    }),
    keymap.of([
      { key: "Mod-s", run: () => {
        opts.onSave();
        return true;
      } },
      indentWithTab,
      ...defaultKeymap,
      ...historyKeymap,
      ...searchKeymap,
    ]),
    EditorView.updateListener.of((update) => {
      opts.onStateChange?.(update.state);
      if (update.docChanged) {
        const saved = opts.savedText();
        opts.onDirtyChange(saved ? !update.state.doc.eq(saved) : false);
      }
      if (update.selectionSet || update.docChanged) {
        const head = update.state.selection.main.head;
        const line = update.state.doc.lineAt(head);
        opts.onCursorChange(line.number, head - line.from + 1);
      }
    }),
  ];
}

/** 创建编辑器状态；selection 为可选光标位置（行/列均从 1 起，超出时夹紧） */
export function createEditorState(
  doc: string,
  extensions: Extension[],
  selection?: { line: number; col: number },
): EditorState {
  const opts: {
    doc: string;
    extensions: Extension[];
    selection?: EditorSelection | SelectionRange;
  } = {
    doc,
    extensions,
  };
  if (selection) {
    opts.selection = EditorSelection.cursor(posAtLineCol(doc, selection.line, selection.col));
  }
  return EditorState.create(opts);
}

/** 行/列（1 起）→ 文档偏移；行不存在时取文档末尾，列超出行尾时取行尾 */
export function posAtLineCol(doc: string, line: number, col: number): number {
  let pos = 0;
  let current = 1;
  while (current < line) {
    const nl = doc.indexOf("\n", pos);
    if (nl < 0) break;
    pos = nl + 1;
    current++;
  }
  const lineEnd = doc.indexOf("\n", pos);
  const end = lineEnd < 0 ? doc.length : lineEnd;
  return Math.min(end, pos + Math.max(0, col - 1));
}
