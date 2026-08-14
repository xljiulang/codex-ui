import { EditorState } from "@codemirror/state";
import { indentRange, indentUnit } from "@codemirror/language";
import type { SyntaxNode } from "@lezer/common";
import { parser as xmlParser } from "@lezer/xml";

import { languageForPath } from "./editorSetup";

/**
 * 编辑器“代码格式化”。
 *
 * 支持范围覆盖 editorSetup.ts 中已注册的全部语言，分三档：
 * - Prettier（动态加载，能展开单行）：JS/TS、JSON/JSONC、CSS、HTML、YAML、Markdown
 * - 自定义 XML 美化（基于 @lezer/xml 语法树重排，能展开单行）：XML/SVG
 * - 缩进重排（复用 CodeMirror 语言包内置缩进规则，仅重排已有换行的缩进）：
 *   Python、Go、Rust、C/C++、Java、Kotlin、C#、PHP、SQL、Shell、PowerShell、Ruby、INI 等
 */

/** Prettier 负责的扩展名（含点、小写）→ 解析器与插件 */
const PRETTIER_EXTS: Record<string, { parser: string; plugin: string }> = {
  js: { parser: "babel", plugin: "estree" },
  jsx: { parser: "babel", plugin: "estree" },
  mjs: { parser: "babel", plugin: "estree" },
  cjs: { parser: "babel", plugin: "estree" },
  ts: { parser: "typescript", plugin: "typescript" },
  mts: { parser: "typescript", plugin: "typescript" },
  cts: { parser: "typescript", plugin: "typescript" },
  tsx: { parser: "typescript", plugin: "typescript" },
  json: { parser: "json", plugin: "estree" },
  jsonc: { parser: "json", plugin: "estree" },
  css: { parser: "css", plugin: "postcss" },
  html: { parser: "html", plugin: "html" },
  htm: { parser: "html", plugin: "html" },
  yml: { parser: "yaml", plugin: "yaml" },
  yaml: { parser: "yaml", plugin: "yaml" },
  md: { parser: "markdown", plugin: "markdown" },
  markdown: { parser: "markdown", plugin: "markdown" },
};

/** 自定义 XML 美化的扩展名 */
const XML_EXTS = new Set(["xml", "svg"]);

/** 缩进重排（非 Prettier、非 XML）的扩展名 */
const INDENT_EXTS = new Set([
  "py",
  "pyw",
  "rs",
  "cs",
  "c",
  "h",
  "cpp",
  "cc",
  "cxx",
  "hpp",
  "hh",
  "hxx",
  "go",
  "java",
  "kt",
  "kts",
  "php",
  "sql",
  "sh",
  "bash",
  "zsh",
  "ps1",
  "psm1",
  "rb",
  "ini",
  "cfg",
  "diff",
  "patch",
]);

/** 编辑器可格式化的全部扩展名（小写） */
export const FORMATABLE_EXTS = new Set<string>([
  ...Object.keys(PRETTIER_EXTS),
  ...XML_EXTS,
  ...INDENT_EXTS,
]);

export type FormatResult =
  | { ok: true; text: string }
  | { ok: true; unchanged: true }
  | { ok: false; message: string };

export function isFormatablePath(path: string): boolean {
  const name = path.split(/[\\/]/).pop() ?? "";
  const dot = name.lastIndexOf(".");
  const ext = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
  return FORMATABLE_EXTS.has(ext);
}

function extOf(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? "";
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

/** 保留原文档换行风格（CRLF/LF）与结尾换行习惯 */
function preserveTrailing(text: string, formatted: string): string {
  const eol = text.includes("\r\n") ? "\r\n" : "\n";
  if (eol === "\n") return formatted;
  return formatted.replace(/\n/g, "\r\n");
}

function withTrailing(text: string, out: string): string {
  const trailing = /\n$/.test(text) ? "\n" : "";
  return trailing ? `${out}\n` : out;
}

function errResult(kind: string): FormatResult {
  return { ok: false, message: `${kind} 语法错误，无法格式化` };
}

/** 模块级 Promise 缓存：Prettier 只首次使用时动态加载 */
let prettierPromise: Promise<{
  format: (text: string, opts: Record<string, unknown>) => Promise<string>;
} | null> | null = null;

async function loadPrettier(): Promise<{
  format: (text: string, opts: Record<string, unknown>) => Promise<string>;
} | null> {
  if (!prettierPromise) {
    prettierPromise = (async () => {
      try {
        const [
          { format },
          babel,
          estree,
          typescript,
          postcss,
          html,
          yaml,
          markdown,
        ] =
          await Promise.all([
            import("prettier/standalone"),
            import("prettier/plugins/babel"),
            import("prettier/plugins/estree"),
            import("prettier/plugins/typescript"),
            import("prettier/plugins/postcss"),
            import("prettier/plugins/html"),
            import("prettier/plugins/yaml"),
            import("prettier/plugins/markdown"),
          ]);
        return {
          format: (text, opts) =>
            format(text, {
              ...opts,
              plugins: [
                babel,
                estree,
                typescript,
                postcss,
                html,
                yaml,
                markdown,
              ],
            }),
        };
      } catch {
        return null;
      }
    })();
  }
  return prettierPromise;
}

async function formatWithPrettier(
  path: string,
  text: string,
): Promise<FormatResult> {
  const ext = extOf(path);
  const conf = PRETTIER_EXTS[ext];
  const prettier = await loadPrettier();
  if (!prettier) {
    return { ok: false, message: "格式化引擎加载失败，请重试" };
  }
  try {
    const out = await prettier.format(text, {
      parser: conf.parser,
      filepath: path,
      tabWidth: 4,
    });
    const eol = text.includes("\r\n") ? "\r\n" : "\n";
    const normalized = eol === "\n" ? out : out.replace(/\n/g, "\r\n");
    const final = /\n$/.test(text) && !/\n$/.test(normalized)
      ? `${normalized}\n`
      : !/\n$/.test(text) && /\n$/.test(normalized)
        ? normalized.replace(/\n$/, "")
        : normalized;
    if (final === text) return { ok: true, unchanged: true };
    return { ok: true, text: final };
  } catch {
    return errResult(conf.parser === "json" ? "JSON" : "代码");
  }
}

function hasErrorNode(parser: typeof xmlParser, text: string): boolean {
  const tree = parser.parse(text);
  let hasError = false;
  tree.iterate({
    enter: (node) => {
      if (
        node.type.isError ||
        node.name === "MissingCloseTag" ||
        node.name === "MismatchedCloseTag"
      ) {
        hasError = true;
        return false;
      }
      return true;
    },
  });
  return hasError;
}

/** XML 文档体必须是元素/注释/处理指令/DOCTYPE 等，裸文本视为无效 */
function hasBareXmlText(text: string): boolean {
  const top = xmlParser.parse(text).topNode;
  for (let n = top.firstChild; n; n = n.nextSibling) {
    if (n.name === "Text" && /\S/.test(text.slice(n.from, n.to))) {
      return true;
    }
  }
  return false;
}

/* ---------------- XML 自定义美化 ---------------- */

const INDENT = "    ";

function hasNonWsText(node: SyntaxNode, text: string): boolean {
  for (let c = node.firstChild; c; c = c.nextSibling) {
    if (c.name === "Text" && /\S/.test(text.slice(c.from, c.to))) {
      return true;
    }
  }
  return false;
}

/** 纯文本/混合内容元素：内容（含内联子元素与空格）原样单行保留 */
function inlineXmlChildren(
  node: SyntaxNode,
  text: string,
  depth: number,
): string {
  let out = "";
  for (let c = node.firstChild; c; c = c.nextSibling) {
    if (c.name === "OpenTag" || c.name === "CloseTag") {
      continue;
    }
    if (c.name === "Text") {
      out += text.slice(c.from, c.to);
    } else if (c.name === "Element") {
      out += formatXmlElement(c, text, depth);
    } else {
      out += text.slice(c.from, c.to);
    }
  }
  return out;
}

/** 仅子元素元素：子元素独占一行并缩进，标签间纯空白丢弃 */
function blockXmlChildren(
  node: SyntaxNode,
  text: string,
  depth: number,
): string {
  const lines: string[] = [];
  for (let c = node.firstChild; c; c = c.nextSibling) {
    if (c.name === "OpenTag" || c.name === "CloseTag") {
      continue;
    }
    if (c.name === "Text") {
      if (/\S/.test(text.slice(c.from, c.to))) lines.push(text.slice(c.from, c.to));
      continue;
    }
    if (c.name === "Element") {
      lines.push(formatXmlElement(c, text, depth));
      continue;
    }
    lines.push(text.slice(c.from, c.to));
  }
  return lines.map((l) => `${INDENT.repeat(depth)}${l}`).join("\n");
}

function formatXmlElement(node: SyntaxNode, text: string, depth: number): string {
  const openTag = node.getChild("OpenTag") ?? node.getChild("SelfClosingTag");
  const open = openTag ? text.slice(openTag.from, openTag.to) : text.slice(node.from, node.to);
  const closeTag = node.getChild("CloseTag");
  const elementChildren = node.getChildren("Element");

  if (!elementChildren.length) {
    // 空元素/纯文本/混合内容：单行保留原文
    const inner = node.firstChild
      ? inlineXmlChildren(node, text, depth)
      : "";
    const close = closeTag ? text.slice(closeTag.from, closeTag.to) : "";
    return `${open}${inner}${close}`;
  }

  const mixed = hasNonWsText(node, text);
  if (mixed || !closeTag) {
    // 混合内容（或残缺结构）：单行保留原文，不再尝试重排
    const inner = inlineXmlChildren(node, text, depth);
    const close = closeTag ? text.slice(closeTag.from, closeTag.to) : "";
    return `${open}${inner}${close}`;
  }

  // 仅子元素：每行缩进；注释/CDATA 等也独占一行
  const inner = blockXmlChildren(node, text, depth + 1);
  const close = closeTag ? text.slice(closeTag.from, closeTag.to) : "";
  const out = `${open}\n${inner}\n${INDENT.repeat(depth)}${close}`;
  return out;
}

function formatXml(text: string): FormatResult {
  if (!text.trim()) return { ok: true, unchanged: true };
  if (hasErrorNode(xmlParser, text) || hasBareXmlText(text)) {
    return errResult("XML");
  }

  const top = xmlParser.parse(text).topNode;
  const out: string[] = [];
  for (let n = top.firstChild; n; n = n.nextSibling) {
    if (n.name === "Text") {
      const raw = text.slice(n.from, n.to);
      if (/\S/.test(raw)) out.push(raw);
      continue;
    }
    if (n.name === "Element") {
      out.push(formatXmlElement(n, text, 0));
      continue;
    }
    out.push(text.slice(n.from, n.to));
  }
  const joined = out.join("\n");
  const result = withTrailing(text, joined);
  if (result === text) return { ok: true, unchanged: true };
  return { ok: true, text: preserveTrailing(text, result) };
}

/* ---------------- 缩进重排（其余语言） ---------------- */

function formatWithIndent(path: string, text: string): FormatResult {
  if (!text.trim()) return { ok: true, unchanged: true };
  const language = languageForPath(path);
  if (!language) return { ok: true, unchanged: true };
  const state = EditorState.create({
    doc: text,
    extensions: [language, indentUnit.of("    ")],
  });
  const changes = indentRange(state, 0, text.length);
  if (changes.empty) return { ok: true, unchanged: true };
  const out = state.update({ changes }).state.doc.toString();
  if (out === text) return { ok: true, unchanged: true };
  return { ok: true, text: out };
}

/** 统一入口：按扩展名路由到 Prettier / XML 美化 / 缩进重排 */
export async function formatDoc(
  path: string,
  text: string,
): Promise<FormatResult> {
  const ext = extOf(path);
  if (PRETTIER_EXTS[ext]) return formatWithPrettier(path, text);
  if (XML_EXTS.has(ext)) return formatXml(text);
  return formatWithIndent(path, text);
}
