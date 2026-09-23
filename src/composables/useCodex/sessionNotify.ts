// useCodex 拆分模块：会话级事件（错误 / 提权交互 / 计划就绪）→ Windows 系统通知
// （仅在窗口没有前台焦点时发出）
//
// 应用内提示不受影响：调用方仍旧写 `store.toast`、渲染会话内错误卡片与交互气泡，
// 这里只额外把「用户看不到窗口时」会错过的事情转投 Windows 通知（系统 toast/操作中心）。
// 焦点判定（含交互通知的不去重策略）与投递都在后端命令 `notify_session_event` 内完成
// （见 commands.rs）。
import { invoke } from "@tauri-apps/api/core";
import { friendlyServerError } from "../../lib/serverMessages";
import { findSessionTabByThread } from "./sessionState";
import { isBackgroundThread, store } from "./store";

/** 通知标题里会话名最大字符数（超出截断，避免标题过长被系统省略） */
const TITLE_SESSION_MAX_CHARS = 40;
/** 通知正文最大字符数（与后端 notifications::MAX_BODY_CHARS 一致） */
const BODY_MAX_CHARS = 200;

/** 通知来源（后端日志 `source=` 与节流策略据此区分） */
export type SessionNotifySource =
  "error" | "interaction" | "plan" | "completion";

/** 需要用户处理的交互类型（决定通知标题） */
export type InteractionKind =
  "approval" | "question" | "elicitation" | "plan" | "other";

/** 单行化（换行/制表/连续空白压成单空格）并按字符数截断，超出追加省略号 */
export function flattenNoticeText(text: string, maxChars: number): string {
  const flat = text.split(/\s+/).filter(Boolean).join(" ");
  const chars = Array.from(flat);
  if (chars.length <= maxChars) return flat;
  return chars.slice(0, maxChars).join("") + "…";
}

/**
 * 会话显示标题：优先已打开的会话标签（标签名可能来自首条消息），其次历史列表摘要；
 * 都取不到返回空串（调用方据此只用固定标题前缀，不显示「新建会话」这类占位文案）。
 */
export function sessionTitleForThread(threadId: string): string {
  const tab = findSessionTabByThread(threadId);
  const tabName = (tab?.name ?? "").trim();
  if (tabName) return tabName;
  const summary = store.threads.find((t) => t.id === threadId);
  return (summary?.name || summary?.preview || "").trim();
}

/** 通知标题：能定位到会话时带上会话名，否则只用固定前缀 */
export function noticeTitle(prefix: string, threadId: string): string {
  const title = sessionTitleForThread(threadId);
  if (!title) return prefix;
  return `${prefix} · ${flattenNoticeText(title, TITLE_SESSION_MAX_CHARS)}`;
}

/** 错误通知正文：错误对象经友好中文映射后单行截断 */
export function errorNoticeBody(
  message: string,
  codexErrorInfo?: unknown,
): string {
  const friendly = friendlyServerError({
    error: { message, codexErrorInfo },
  });
  return flattenNoticeText(friendly, BODY_MAX_CHARS);
}

/** 交互通知的固定文案：正文为通用说明，不含命令/问题/表单原文 */
const INTERACTION_NOTICE: Record<
  InteractionKind,
  { prefix: string; body: string }
> = {
  approval: { prefix: "需要审批", body: "会话正在等待你批准操作" },
  question: { prefix: "需要输入", body: "会话正在等待你回答问题" },
  elicitation: { prefix: "MCP 表单", body: "MCP 工具正在等待你填写表单" },
  plan: { prefix: "计划已就绪", body: "会话已产出计划，等待你确认是否执行" },
  other: { prefix: "需要确认", body: "会话正在等待你的确认" },
};

/** 协议 method → 交互类型；未列举（含未来新增）的方法归入 other */
export function interactionKindForMethod(method: string): InteractionKind {
  switch (method) {
    case "item/commandExecution/requestApproval":
    case "execCommandApproval":
    case "item/fileChange/requestApproval":
    case "applyPatchApproval":
    case "item/permissions/requestApproval":
      return "approval";
    case "item/tool/requestUserInput":
      return "question";
    case "mcpServer/elicitation/request":
      return "elicitation";
    default:
      return "other";
  }
}

export interface SessionErrorNotice {
  /** 错误正文（原始 message；内部按 codexErrorInfo → 文本规则映射为友好中文） */
  message: string;
  /** 服务端结构化错误信息（可选），命中时优先于文本匹配 */
  codexErrorInfo?: unknown;
  /** 错误归属线程；缺失时不发系统通知（无法定位会话） */
  threadId?: string | null;
  /** 错误归属回合（可选），后端据此对同一次失败去重 */
  turnId?: string | null;
}

export interface SessionInteractionNotice {
  /** 交互类型（标题按此取固定文案） */
  kind: InteractionKind;
  /** 交互归属线程；缺失时不发系统通知（无法定位会话） */
  threadId?: string | null;
}

export interface SessionCompletedNotice {
  /** 回合归属线程；缺失时不发系统通知（无法定位会话） */
  threadId?: string | null;
}

/**
 * 通知的统一门槛与投递（fire-and-forget）：
 * 开关关闭 / 无 threadId / 归属后台临时线程（标题总结的 ephemeral 线程，不打扰用户且
 * 通知按钮也定位不到会话）/ 标题或正文为空 —— 任一命中即静默返回。
 */
function sendSessionNotice(opts: {
  enabled: boolean;
  title: string;
  body: string;
  threadId: string;
  turnId?: string;
  source: SessionNotifySource;
}): void {
  if (!opts.enabled || !opts.threadId || !opts.title || !opts.body) return;
  const turnId = (opts.turnId ?? "").trim();
  void invoke("notify_session_event", {
    title: opts.title,
    body: opts.body,
    threadId: opts.threadId,
    turnId: turnId || null,
    source: opts.source,
  }).catch(() => {});
}

/**
 * codex 产生的会话错误 → Windows 通知（未聚焦时）。
 * 开关：设置项 `error_notify_enabled`。
 */
export function notifySessionError(notice: SessionErrorNotice): void {
  const threadId = (notice.threadId ?? "").trim();
  if (!threadId || isBackgroundThread(threadId) || !notice.message.trim())
    return;
  const body = errorNoticeBody(notice.message, notice.codexErrorInfo);
  sendSessionNotice({
    enabled: !!store.settings.error_notify_enabled,
    title: noticeTitle("会话错误", threadId),
    body,
    threadId,
    turnId: notice.turnId ?? undefined,
    source: "error",
  });
}

/**
 * 会话等待人工处理（审批 / 提问 / MCP 表单 / 计划就绪）→ Windows 通知（未聚焦时）。
 * 开关：设置项 `interaction_notify_enabled`。每条交互请求都发（后端不做去重），
 * 避免用户切走后漏掉需要立即处理的确认。
 */
export function notifySessionInteraction(
  notice: SessionInteractionNotice,
): void {
  const threadId = (notice.threadId ?? "").trim();
  if (!threadId || isBackgroundThread(threadId)) return;
  const text = INTERACTION_NOTICE[notice.kind] ?? INTERACTION_NOTICE.other;
  sendSessionNotice({
    enabled: !!store.settings.interaction_notify_enabled,
    title: noticeTitle(text.prefix, threadId),
    body: text.body,
    threadId,
    source: notice.kind === "plan" ? "plan" : "interaction",
  });
}

/**
 * 会话正常完成（`turn/completed` 且 status=completed）→ Windows 通知（未聚焦时）。
 * 开关：设置项 `interaction_notify_enabled`（与提权/交互共用）。
 * 不带 turnId（回合已正常收尾，没有需要合并的双报）；不传正文，固定通用文案。
 */
export function notifySessionCompleted(notice: SessionCompletedNotice): void {
  const threadId = (notice.threadId ?? "").trim();
  if (!threadId || isBackgroundThread(threadId)) return;
  sendSessionNotice({
    enabled: !!store.settings.interaction_notify_enabled,
    title: noticeTitle("会话完成", threadId),
    body: "会话已完成，可以查看结果",
    threadId,
    source: "completion",
  });
}
