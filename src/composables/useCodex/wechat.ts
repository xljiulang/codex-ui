// useCodex 拆分模块：微信接入（ClawBot）状态与动作。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { WeChatSnapshot } from "../../lib/types";
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

/** 发起扫码登录：二维码内容随后经 wechat/event 推送。 */
export async function wechatLoginStart(): Promise<void> {
  await ensureWeChatEvents();
  await invoke("wechat_login_start");
}

/** 退出登录：清除本地凭据并回到未登录态。 */
export async function wechatLogout(): Promise<void> {
  await ensureWeChatEvents();
  await invoke("wechat_logout");
  await refreshWeChatState();
}

/** 设置变更后的运行时对齐：按 settings.json 的开关启停 sidecar。 */
export async function wechatServiceSync(): Promise<void> {
  await ensureWeChatEvents();
  await invoke("wechat_service_sync");
  await refreshWeChatState();
}
