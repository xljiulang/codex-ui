import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { askConfirm, interrupt, store } from "./useCodex";

/**
 * 主窗口关闭守卫：当前回合进行中时先让用户确认，确认后停止回合再关闭窗口。
 * 与“切换会话”共用同一套全局确认框；非 Tauri 环境（浏览器/单测）返回 no-op，
 * 避免注册过程抛错。
 */
export async function registerCloseGuard(): Promise<UnlistenFn> {
  try {
    const win = getCurrentWindow();
    return await win.onCloseRequested(async (event) => {
      // 回合空闲：允许默认关闭
      if (!(store.turnActive && store.currentThreadId)) return;
      event.preventDefault();
      // 已有确认框（如切换会话弹窗）时不叠加，仅阻止关闭
      if (store.confirm) return;
      const ok = await askConfirm({
        title: "关闭应用",
        message: "当前对话仍在进行中，关闭将停止当前回合。是否继续？",
        confirmLabel: "停止并关闭",
        cancelLabel: "取消",
      });
      if (!ok) return;
      // 先停止当前回合（与切换会话一致，含目标模式清目标），再强制关闭，
      // destroy 不会再次触发 close-requested，避免循环。
      await interrupt(store.currentThreadId, store.currentTurnId);
      await win.destroy();
    });
  } catch {
    return () => {};
  }
}
