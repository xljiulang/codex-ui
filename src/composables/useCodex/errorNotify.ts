// useCodex 拆分模块：codex 错误 → Windows 系统通知（仅在窗口没有前台焦点时发出）
//
// 应用内提示不受影响：调用方仍旧写 `store.toast` / 渲染会话内错误卡片，这里只额外
// 把「用户看不到窗口时」会错过的 error 级内容转投 Windows 通知（系统 toast/操作中心）。
// 焦点判定、节流与投递都在后端命令 `notify_codex_error` 内完成（见 commands.rs）。
import { invoke } from "@tauri-apps/api/core";
import { friendlyServerError } from "../../lib/serverMessages";
import { findSessionTabByThread } from "./sessionState";
import { isBackgroundThread, store } from "./store";


/** 通知标题里会话名最大字符数（超出截断，避免标题过长被系统省略） */
const TITLE_SESSION_MAX_CHARS = 40;
/** 通知正文最大字符数（与后端 notifications::MAX_BODY_CHARS 一致） */
const BODY_MAX_CHARS = 200;


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
export function codexErrorNoticeTitle(threadId: string): string {
  const title = sessionTitleForThread(threadId);
  if (!title) return "Codex 错误";
  return `Codex 错误 · ${flattenNoticeText(title, TITLE_SESSION_MAX_CHARS)}`;
}


/** 通知正文：错误对象经友好中文映射后单行截断 */
export function codexErrorNoticeBody(message: string, codexErrorInfo?: unknown): string {
  const friendly = friendlyServerError({
    error: { message, codexErrorInfo },
  });
  return flattenNoticeText(friendly, BODY_MAX_CHARS);
}


export interface CodexErrorNotice {
  /** 错误正文（原始 message；内部按 codexErrorInfo → 文本规则映射为友好中文） */
  message: string;
  /** 服务端结构化错误信息（可选），命中时优先于文本匹配 */
  codexErrorInfo?: unknown;
  /** 错误归属线程；缺失时不发系统通知（无法定位会话） */
  threadId?: string | null;
  /** 错误归属回合（可选），后端据此对同一次失败去重 */
  turnId?: string | null;
}


/**
 * 把 codex 错误同时投到 Windows 通知（未聚焦时）。
 *
 * 关闭条件：设置项 `error_notify_enabled` 关闭、无 threadId、正文为空、归属后台临时线程
 * （如标题总结的 ephemeral 线程，其错误不打扰用户且通知按钮也定位不到会话）——任一命中即返回。
 * 调用方不需要 await（fire-and-forget）：失败只影响通知本身，绝不影响应用内提示与主流程。
 */
export function notifyCodexError(notice: CodexErrorNotice): void {
  if (!store.settings.error_notify_enabled) return;
  const threadId = (notice.threadId ?? "").trim();
  if (!threadId || isBackgroundThread(threadId) || !notice.message.trim()) return;
  const body = codexErrorNoticeBody(notice.message, notice.codexErrorInfo);
  if (!body) return;
  const title = codexErrorNoticeTitle(threadId);
  const turnId = (notice.turnId ?? "").trim();
  void invoke("notify_codex_error", {
    title,
    body,
    threadId,
    turnId: turnId || null,
  }).catch(() => {});
}
