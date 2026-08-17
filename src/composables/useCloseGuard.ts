import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { askConfirm, interrupt, store } from "./useCodex";
import { dirtyEditableTabs, saveAllDirtyTabs, tabs } from "./useEditorTabs";
import { isTabWorking, TabKind } from "../lib/tabs";
import type { SessionTab } from "./useCodex";
import type { TerminalEditorTab } from "./useEditorTabs";

/**
 * 主窗口关闭守卫：仍有工作中的标签（会话回合/目标续跑、终端命令执行）时先让用户确认，
 * 确认后停止全部工作会话并终止运行中终端，再关闭窗口。
 * 与“切换会话”共用同一套全局确认框；非 Tauri 环境（浏览器/单测）返回 no-op，
 * 避免注册过程抛错。
 */
export async function registerCloseGuard(): Promise<UnlistenFn> {
  try {
    const win = getCurrentWindow();
    return await win.onCloseRequested(async (event) => {
      // 工作中的标签：先确认停止再关闭（会话含目标激活续跑；终端含命令执行中）
      const workingTabs = tabs.filter((t) => isTabWorking(t));
      const workingSessions = workingTabs.filter(
        (t): t is SessionTab => t.kind === TabKind.Chat,
      );
      const busyTerminals = workingTabs.filter(
        (t): t is TerminalEditorTab => t.kind === TabKind.Terminal,
      );
      if (workingSessions.length > 0 || busyTerminals.length > 0) {
        event.preventDefault();
        // 已有确认框（如切换会话弹窗）时不叠加，仅阻止关闭
        if (store.confirm) return;
        const ok = await askConfirm({
          title: "关闭应用",
          message: `有 ${workingSessions.length} 个会话、${busyTerminals.length} 个终端正在工作，关闭将停止它们。是否继续？`,
          confirmLabel: "停止并关闭",
          cancelLabel: "取消",
        });
        if (!ok) return;
        // 停止所有工作中的会话（含活跃目标清目标），终止所有运行中终端
        await Promise.all(
          workingSessions.map((s) =>
            s.threadId
              ? interrupt(s.threadId, s.currentTurnId)
              : Promise.resolve(),
          ),
        );
        await Promise.all(
          busyTerminals.map((t) =>
            invoke("terminal_kill", { id: t.id }).catch(() => undefined),
          ),
        );
        // destroy 不会再次触发 close-requested，避免循环。
        await win.destroy();
        return;
      }
      // 存在未保存文件：确认先保存再关闭，避免丢失编辑内容
      const dirty = dirtyEditableTabs();
      if (dirty.length > 0) {
        event.preventDefault();
        if (store.confirm) return;
        const ok = await askConfirm({
          title: "关闭应用",
          message: `有 ${dirty.length} 个文件未保存，关闭将丢失这些更改。是否先保存再关闭？`,
          confirmLabel: "保存并关闭",
          cancelLabel: "取消",
        });
        if (!ok) return;
        // 任一文件保存失败则不关闭，保留未保存内容
        if (!(await saveAllDirtyTabs())) return;
        await win.destroy();
      }
    });
  } catch {
    return () => {};
  }
}
