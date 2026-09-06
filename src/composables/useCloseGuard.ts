import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { flushLastSession, interrupt } from "./useCodex";
import { tabs } from "./useEditorTabs";
import { isTabWorking, TabKind } from "../lib/tabs";
import type { SessionTab } from "./useCodex";
import type { TerminalEditorTab } from "./useEditorTabs";

/**
 * 应用退出守卫：
 * - 窗口 X 关闭：仅 `event.preventDefault()` 阻止 Tauri 默认销毁窗口，交由 Rust 端隐藏到系统托盘。
 * - 托盘「退出」（`app-exit-requested`）：静默收尾——停止工作会话（会话含清目标）并终止运行中终端，
 *   把最后活跃会话 id 落盘到配置文件，随后调用后端 `app_exit` 触发 `RunEvent::Exit` 统一关停；
 *   不保存其它未保存内容（随进程结束丢弃）。
 *
 * 非 Tauri 环境（浏览器/单测）返回 no-op，避免注册过程抛错。
 */
export async function registerCloseGuard(): Promise<UnlistenFn> {
  try {
    const win = getCurrentWindow();
    const unlistenClose = await win.onCloseRequested(async (event) => {
      event.preventDefault();
    });
    const unlistenExit = await listen("app-exit-requested", async () => {
      const workingTabs = tabs.filter((t) => isTabWorking(t));
      try {
        await Promise.all(
          workingTabs
            .filter((t): t is SessionTab => t.kind === TabKind.Session)
            .map((s) =>
              s.threadId
                ? interrupt(s.threadId, s.currentTurnId)
                : Promise.resolve(),
            ),
        );
        await Promise.all(
          workingTabs
            .filter((t): t is TerminalEditorTab => t.kind === TabKind.Terminal)
            .map((t) =>
              invoke("terminal_kill", { id: t.id }).catch(() => undefined),
            ),
        );
      } catch {
        // 中断/终止终端的异常在此吞掉，避免未处理的 Promise rejection；不阻断退出。
      } finally {
        // 最后活跃会话 id 落盘（内部吞错，不阻断退出），再触发 app_exit；
        // 退出调用失败交给后端兜底超时。
        await flushLastSession();
        try {
          await invoke("app_exit");
        } catch {
          // 忽略 app_exit 失败。
        }
      }
    });
    return () => {
      unlistenClose();
      unlistenExit();
    };
  } catch {
    return () => {};
  }
}
