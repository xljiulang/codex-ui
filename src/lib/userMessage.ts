import { marked } from "marked";
import { FILE_MENTION_HEADING, MY_REQUEST_MARKER } from "./mention";
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

function tableCellText(cell: unknown): string {
  const c = cell as MarkdownWalkToken;
  return tokenListText(tokenChildren(c.tokens)) || tokenTextString(c.text);
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

/** html token 的可见文本近似：去掉标签，空结果由上层过滤 */
function htmlVisibleText(html: string): string {
  return html.replace(/<[^>]*>/g, " ");
}

/** 单 token → 文本：按块/行内结构递归取用户可见内容（代码块与任务勾选不计入） */
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
      return tokenListText(tokenChildren(t.tokens)) || tokenTextString(t.text);
    case "html":
      return htmlVisibleText(tokenTextString(t.text));
    default:
      // 行内 token（text/escape/codespan/strong/em/del/link/image/autolink 等）：
      // 优先取子 token 文本，其次取自身 text（link 链接文字与 image alt 同源）
      return tokenListText(tokenChildren(t.tokens)) || tokenTextString(t.text);
  }
}

function collapseWhitespace(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

/**
 * 把 Markdown 源文本转为纯文本（不渲染 HTML）：用与气泡渲染同一引擎（marked）的
 * lexer 分词后递归取文本，结构语义与展示一致；代码块不进入预览，行内代码保留。
 */
export function markdownToPlainText(text: string): string {
  let tokens: readonly MarkdownWalkToken[];
  try {
    tokens = marked.lexer(text, { gfm: true, breaks: true }) as unknown as readonly MarkdownWalkToken[];
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
  for (const c of items) {
    if (c.type === "text") {
      const body = textItemBody(c.text);
      textParts.push(body);
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
  const navText = navParts.join(" ");
  const trimmed = text.trimStart();
  const isExecutePlan =
    trimmed.toUpperCase().startsWith(EXECUTE_PLAN_PREFIX);
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
