import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { setToast, toastError } from "../composables/useCodex";
import type { FileEditorTab } from "../composables/useEditorTabs";
import hljs from "./highlight";
import { renderMarkdown } from "./markdownRenderer";

/** Windows 风格路径解析：处理 . 与 ..，不做文件系统访问 */
function resolvePath(base: string, rel: string): string {
  const norm = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "");
  const parts = [...norm(base).split("/"), ...norm(rel).split("/")];
  const out: string[] = [];
  for (const part of parts) {
    if (!part || part === ".") continue;
    if (part === "..") out.pop();
    else out.push(part);
  }
  return out.join("/");
}

/** 绝对路径的所在目录（保留盘符与分隔风格） */
function dirOf(absPath: string): string {
  const idx = Math.max(absPath.lastIndexOf("\\"), absPath.lastIndexOf("/"));
  return idx > 0 ? absPath.slice(0, idx) : absPath;
}

/** 去掉目录与 .md/.markdown 扩展名后的文件名 */
function baseNameOf(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? path;
  return name.replace(/\.(md|markdown)$/i, "");
}

/** 默认导出文件名：<md 文件名>.pdf */
export function pdfSuggestedName(path: string): string {
  return `${baseNameOf(path)}.pdf`;
}

/**
 * 解析 markdown 图片 src：
 * - http(s)/data/asset 原样保留；
 * - file://、盘符绝对路径 → 本地路径转 asset URL；
 * - 相对路径 → 相对 md 文件所在目录解析后转 asset URL。
 * 返回 null 表示无法作为图片（空/锚点/未知 scheme）。
 */
export function resolveImageSrc(
  src: string,
  fileAbsPath: string,
): string | null {
  let h = src.trim();
  if (!h) return null;
  try {
    h = decodeURIComponent(h);
  } catch {
    // 保留原值
  }
  if (/^https?:\/\//i.test(h)) return h;
  if (/^data:/i.test(h)) return h;
  if (/^asset:/i.test(h)) return h;

  let local = h;
  if (local.startsWith("file:///")) local = local.slice("file:///".length);
  if (/^\/[A-Za-z]:\//.test(local)) local = local.slice(1);
  if (/^[A-Za-z]:[\\/]/.test(local)) {
    return convertFileSrc(local);
  }
  if (local.startsWith("#")) return null;
  if (/^[A-Za-z][A-Za-z0-9+.-]*:/.test(local)) return null;
  return convertFileSrc(resolvePath(dirOf(fileAbsPath), local));
}

/** 导出 PDF 内联打印样式：A4 纵向、16mm 边距、浅色主题、hljs 浅色 token 色 */
const PRINT_CSS = `
@page { size: A4; margin: 16mm; }
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; }
body {
  font-family: -apple-system, "Segoe UI", "Microsoft YaHei", "PingFang SC", "Noto Sans CJK SC", sans-serif;
  font-size: 14px;
  line-height: 1.65;
  color: #1f2328;
  background: #ffffff;
}
.md { max-width: 100%; }
.md p { margin: 0 0 8px; }
.md h1, .md h2, .md h3, .md h4, .md h5, .md h6 {
  margin: 16px 0 8px;
  line-height: 1.3;
  font-weight: 700;
  color: #1f2328;
}
.md h1 { font-size: 22px; border-bottom: 1px solid #d0d7de; padding-bottom: 6px; }
.md h2 { font-size: 18px; border-bottom: 1px solid #d8dee4; padding-bottom: 4px; }
.md h3 { font-size: 16px; }
.md ul, .md ol { padding-left: 22px; margin: 0 0 8px; }
.md li { margin: 2px 0; }
.md a { color: #0969da; text-decoration: none; }
.md blockquote {
  margin: 8px 0;
  padding: 4px 14px;
  color: #57606a;
  border-left: 4px solid #d0d7de;
  page-break-inside: avoid;
}
.md hr { border: none; border-top: 1px solid #d0d7de; margin: 16px 0; }
.md img { max-width: 100%; }
.md code {
  font-family: Consolas, "Cascadia Code", "Courier New", monospace;
  font-size: 12.5px;
  background: rgba(175, 184, 193, 0.2);
  padding: 1px 5px;
  border-radius: 4px;
}
.md pre {
  background: #f6f8fa;
  border: 1px solid #d0d7de;
  border-radius: 6px;
  padding: 12px;
  overflow-x: auto;
  margin: 8px 0;
  page-break-inside: avoid;
}
.md pre code {
  background: none;
  padding: 0;
  font-size: 12.5px;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-all;
}
.md table { border-collapse: collapse; width: 100%; margin: 8px 0; page-break-inside: avoid; }
.md th, .md td { border: 1px solid #d0d7de; padding: 6px 10px; text-align: left; }
.md th { background: #f6f8fa; font-weight: 600; }
.md tr:nth-child(even) td { background: #fafbfc; }
.md input[type="checkbox"] { margin-right: 4px; }
/* highlight.js 浅色 token 色（github light 风格） */
.hljs-comment, .hljs-quote { color: #6e7781; font-style: italic; }
.hljs-keyword, .hljs-selector-tag, .hljs-subst { color: #cf222e; }
.hljs-number, .hljs-literal, .hljs-variable, .hljs-template-variable, .hljs-tag .hljs-attr { color: #0550ae; }
.hljs-string, .hljs-doctag { color: #0a3069; }
.hljs-title, .hljs-section, .hljs-selector-id { color: #8250df; }
.hljs-type, .hljs-class .hljs-title, .hljs-title.class_ { color: #116329; }
.hljs-built_in, .hljs-builtin-name { color: #953800; }
.hljs-meta { color: #57606a; }
.hljs-attr, .hljs-attribute { color: #0550ae; }
.hljs-symbol, .hljs-bullet, .hljs-link { color: #0a3069; }
.hljs-emphasis { font-style: italic; }
.hljs-strong { font-weight: 600; }
`;

/**
 * 把 markdown 渲染为自包含的打印 HTML：
 * 复用现有 marked 渲染链路，追加代码高亮、本地图片 asset 化与浅色打印样式。
 */
export async function buildPrintHtml(
  text: string,
  fileAbsPath: string,
): Promise<string> {
  const raw = await renderMarkdown(text);
  const doc = new DOMParser().parseFromString(
    `<div class="md">${raw}</div>`,
    "text/html",
  );

  // 代码块语法高亮（沿用预览同款 hljs 语言集）
  for (const code of Array.from(
    doc.querySelectorAll("pre code[class*='language-']"),
  )) {
    const lang = /language-([\w-]+)/.exec(code.className)?.[1] ?? "";
    if (lang && hljs.getLanguage(lang)) {
      try {
        code.innerHTML = hljs.highlight(code.textContent ?? "", {
          language: lang,
        }).value;
        code.classList.add("hljs");
      } catch {
        // 高亮失败保持原文
      }
    }
  }

  // 本地图片转 asset URL，远程/data 保留
  for (const img of Array.from(doc.querySelectorAll("img"))) {
    const src = img.getAttribute("src");
    if (!src) continue;
    const resolved = resolveImageSrc(src, fileAbsPath);
    if (resolved) img.setAttribute("src", resolved);
  }

  const body = doc.body.querySelector(".md")?.outerHTML ?? "";
  return `<!doctype html><html lang="zh-CN"><head><meta charset="UTF-8"><title>PDF 导出</title><style>${PRINT_CSS}</style></head><body>${body}</body></html>`;
}

/**
 * 右键菜单「导出 PDF」入口：以当前（含未保存）文档内容生成打印 HTML，
 * 交给后端保存对话框 + WebView2 静默写出 PDF。
 */
export async function exportMarkdownToPdf(tab: FileEditorTab): Promise<void> {
  const text = tab.editorState?.doc.toString() ?? "";
  if (!text.trim()) {
    setToast("文档为空，无法导出 PDF");
    return;
  }
  const fileAbsPath = /^[A-Za-z]:[\\/]/.test(tab.path)
    ? tab.path
    : `${tab.workspace.replace(/[\\/]+$/, "")}\\${tab.path}`;
  const html = await buildPrintHtml(text, fileAbsPath);
  try {
    const saved = await invoke<string | null>("export_markdown_pdf", {
      html,
      suggestedName: pdfSuggestedName(tab.path),
      initialDir: dirOf(fileAbsPath),
    });
    if (saved) setToast(`已导出 PDF：${saved}`);
  } catch (e) {
    setToast(toastError(e));
  }
}
