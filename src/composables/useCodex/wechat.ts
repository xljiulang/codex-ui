// useCodex 拆分模块：微信接入（ClawBot）会话绑定状态与动作。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { WeChatBindingInfo, WeChatSnapshot } from "../../lib/types";
import { store } from "./store";

let unlisten: UnlistenFn | null = null;

/** 注册 wechat/event 全局监听（幂等）：状态全量快照直接替换 store.wechat */
export async function ensureWeChatEvents(): Promise<void> {
  if (unlisten) return;
  unlisten = await listen<WeChatSnapshot>("wechat/event", (e) => {
    store.wechat = e.payload;
  });
}

/** 拉取一次微信桥状态快照。 */
export async function refreshWeChatState(): Promise<void> {
  try {
    const snap = await invoke<WeChatSnapshot | null>("wechat_state");
    // 空结果（后端未就绪）不覆盖既有快照，避免设置页状态闪回「未连接」。
    if (snap) store.wechat = snap;
  } catch {
    // 后端不可用时保持旧值，设置页按 null 渲染占位
  }
}

/** 对指定会话发起扫码绑定：二维码内容随后经 wechat/event 推送。 */
export async function wechatBindLoginStart(threadId: string): Promise<void> {
  await ensureWeChatEvents();
  await invoke("wechat_bind_login_start", { threadId });
  await refreshWeChatState();
}

/** 解除指定会话的微信绑定并停止对应账号接收。 */
export async function wechatUnbind(threadId: string): Promise<void> {
  await ensureWeChatEvents();
  await invoke("wechat_unbind", { threadId });
  await refreshWeChatState();
}

/** 取消当前扫码绑定（弹窗关闭时调用）：释放单条 pending，避免阻塞其它会话绑定。 */
export async function wechatCancelBind(): Promise<void> {
  await ensureWeChatEvents();
  try {
    await invoke("wechat_cancel_bind");
  } catch {
    // 取消失败不阻断关闭
  }
  await refreshWeChatState();
}

/** 指定会话是否已绑定微信（基于最新快照）。 */
export function isThreadBound(threadId: string): boolean {
  return (store.wechat?.bindings ?? []).some((b) => b.threadId === threadId);
}

/** 指定会话的绑定信息（未绑定返回 null）。 */
export function bindingOfThread(threadId: string): WeChatBindingInfo | null {
  return (store.wechat?.bindings ?? []).find((b) => b.threadId === threadId) ?? null;
}
