// useCodex 拆分模块：全局确认弹窗（原 useCodex.ts 的一部分，纯移动，行为不变）
import { store } from "./store";
import type { ConfirmRequest } from "./types";


/** 弹出全局确认框，返回用户选择（true=确认） */
export function askConfirm(req: ConfirmRequest): Promise<boolean> {
  return new Promise((resolve) => {
    store.confirm = { ...req, resolve };
  });
}


/** 用户做出选择后关闭确认框并回传结果 */
export function settleConfirm(ok: boolean) {
  const c = store.confirm;
  if (!c) return;
  store.confirm = null;
  c.resolve(ok);
}
