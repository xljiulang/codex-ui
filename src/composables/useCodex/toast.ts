// useCodex 拆分模块：toast 提示（原 useCodex.ts 的一部分，纯移动，行为不变）
import { watch } from "vue";
import { friendlyServerError } from "../../lib/serverMessages";
import { store } from "./store";


// 任何地方给 store.toast 赋值都会在 5 秒后自动消失
let toastTimer: number | undefined;

watch(
  () => store.toast,
  (v) => {
    if (toastTimer) window.clearTimeout(toastTimer);
    if (v) {
      toastTimer = window.setTimeout(() => {
        store.toast = "";
      }, 5000);
    }
  },
);


export function setToast(msg: string) {
  store.toast = msg;
}


/** 把错误对象转成可读提示：优先提取 error.error.message / error.message，
 * 再经服务端消息映射为友好中文；未匹配保留原文（避免显示原始 JSON） */
export function toastError(e: unknown): string {
  return friendlyServerError(e);
}
