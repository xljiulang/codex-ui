import { lexMarkdown } from "./markdownEngine";
import {
  FILE_MENTION_HEADING,
  MY_REQUEST_MARKER,
  parseFileMentionSection,
} from "./mention";
import { splitPlanTitle } from "./planText";
import {
  isThreadItemType,
  isUserInput,
  type ThreadItem,
  type UserInput,
  type UserMessageItem,
  type UserMessageSummary,
} from "./types";

/** 执行计划消息前缀：用户端「执行计划」按钮发送的执行指令，忽略大小写匹配 */
export const EXECUTE_PLAN_PREFIX = "PLEASE IMPLEMENT THIS PLAN:";

/** 提取用 token 的结构化访问视图（仅取本实现需要的字段） */
interface MarkdownWalkToken {
  type?: string;
  text?: unknown;
  tokens?: unknown;
  items?: unknown;
  header?: unknown;
  rows?: unknown;
}

function tokenTextString(v: unknown): string {
  return typeof v === "string" ? v : "";
}

function tokenChildren(v: unknown): MarkdownWalkToken[] {
  return Array.isArray(v) ? (v as MarkdownWalkToken[]) : [];
}

function tokenListText(list: readonly MarkdownWalkToken[]): string {
  const parts: string[] = [];
  for (const t of list) {
    const s = tokenText(t);
    if (s) parts.push(s);
  }
  return parts.join(" ");
}

/** 行内 token 列表：直接拼接（相邻行内内容本就无分隔，如 `a<b>c`、`<span>x</span> 后缀`） */
function inlineListText(list: readonly MarkdownWalkToken[]): string {
  let out = "";
  for (const t of list) out += tokenText(t);
  return out;
}

function tableCellText(cell: unknown): string {
  const c = cell as MarkdownWalkToken;
  return inlineListText(tokenChildren(c.tokens)) || tokenTextString(c.text);
}

function tableText(t: MarkdownWalkToken): string {
  const rows: unknown[] = [t.header, ...tokenChildren(t.rows)];
  const parts: string[] = [];
  for (const row of rows) {
    const cells = tokenChildren(row)
      .map(tableCellText)
      .filter(Boolean)
      .join(" ");
    if (cells) parts.push(cells);
  }
  return parts.join(" ");
}

/**
 * 单 token → 文本：按块/行内结构递归取用户可见内容（代码块与任务勾选不计入）。
 * html token 取原文——气泡里原始 HTML 已按字面文本显示（见 markdownEngine），
 * 这里保持一致，否则会出现「气泡显示 `<ABCD>` 而导航预览空白」。
 */
function tokenText(t: MarkdownWalkToken): string {
  switch (t.type) {
    case "space":
    case "hr":
    case "def":
    case "code": // fenced/缩进代码块不进入导航预览
      return "";
    case "checkbox":
      return "";
    case "br":
      return " ";
    case "list":
      return tokenListText(tokenChildren(t.items));
    case "list_item":
      return tokenListText(tokenChildren(t.tokens)) || tokenTextString(t.text);
    case "blockquote":
      return tokenListText(tokenChildren(t.tokens));
    case "table":
      return tableText(t);
    case "heading":
    case "lheading":
    case "paragraph":
    case "text":
      return inlineListText(tokenChildren(t.tokens)) || tokenTextString(t.text);
    case "html":
      return tokenTextString(t.text);
    default:
      // 行内 token（escape/codespan/strong/em/del/link/image/autolink 等）：
      // 优先取子 token 文本，其次取自身 text（link 链接文字与 image alt 同源）
      return inlineListText(tokenChildren(t.tokens)) || tokenTextString(t.text);
  }
}

function collapseWhitespace(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

/**
 * 把 Markdown 源文本转为纯文本（不渲染 HTML）：用与气泡渲染同一引擎的
 * lexer 分词后递归取文本，结构语义与展示一致；代码块不进入预览，行内代码保留。
 */
export function markdownToPlainText(text: string): string {
  let tokens: readonly MarkdownWalkToken[];
  try {
    tokens = lexMarkdown(text) as unknown as readonly MarkdownWalkToken[];
  } catch {
    return collapseWhitespace(text);
  }
  return collapseWhitespace(tokenListText(tokens));
}

/** 文本项正文：含 Files 段时取 `## My request:` 之后，否则整段 */
export function textItemBody(text: string): string {
  const marker = `\n${MY_REQUEST_MARKER}\n`;
  const idx = text.indexOf(marker);
  if (idx >= 0 && text.includes(FILE_MENTION_HEADING)) {
    return text.slice(idx + marker.length);
  }
  return text;
}

/**
 * 计算用户消息派生摘要（纯函数，写入点按 content 调用一次并持久化到消息项）。
 * text 与原气泡复制文本逐字节一致：非 text 项以空串参与换行拼接；
 * navText 按内容顺序以占位符表达图片/引用/技能。
 */
export function summarizeUserMessage(
  content: readonly UserInput[] | null | undefined,
): UserMessageSummary {
  const items = Array.isArray(content) ? content.filter(isUserInput) : [];
  const textParts: string[] = [];
  const navParts: string[] = [];
  // 仅当整条消息提取不到任何文本/占位时，降级为 Files 段文件名（@name，与气泡 chip 同源）
  const fileFallbackParts: string[] = [];
  for (const c of items) {
    if (c.type === "text") {
      const body = textItemBody(c.text);
      textParts.push(body);
      for (const f of parseFileMentionSection(c.text)) {
        fileFallbackParts.push(`@${f.name}`);
      }
      const plain = markdownToPlainText(body);
      if (plain) navParts.push(plain);
      continue;
    }
    textParts.push("");
    if (c.type === "localImage") navParts.push("[图片]");
    else if (c.type === "mention") navParts.push(`@${c.name}`);
    else if (c.type === "skill") navParts.push(`$${c.name}`);
  }
  const text = textParts.join("\n");
  const navText = navParts.join(" ") || fileFallbackParts.join(" ");
  const trimmed = text.trimStart();
  const isExecutePlan = trimmed.toUpperCase().startsWith(EXECUTE_PLAN_PREFIX);
  const executePlanText = isExecutePlan
    ? trimmed.slice(EXECUTE_PLAN_PREFIX.length).trim()
    : "";
  const planTitle = isExecutePlan ? splitPlanTitle(executePlanText).title : "";
  return { text, navText, isExecutePlan, executePlanText, planTitle };
}

/** 取用户消息摘要：优先读入库持久化的 derived，缺省（旧数据/直接构造）即时回退计算 */
export function getUserMessageSummary(item: ThreadItem): UserMessageSummary {
  if (!isThreadItemType<UserMessageItem>(item, "userMessage")) {
    return summarizeUserMessage(undefined);
  }
  return item.derived ?? summarizeUserMessage(item.content);
}

/** 在用户消息项上写入派生摘要（写入点统一调用；非用户消息不动） */
export function enrichUserMessage(item: ThreadItem): ThreadItem {
  if (isThreadItemType<UserMessageItem>(item, "userMessage")) {
    item.derived = summarizeUserMessage(item.content);
  }
  return item;
}
