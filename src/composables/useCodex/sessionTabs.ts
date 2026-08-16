// useCodex 拆分模块：会话标签高层操作（原 useCodex.ts 的一部分，纯移动，行为不变）
import { TabKind } from "../../lib/tabs";
import type { UserInput } from "../../lib/types";
import { activeTabId, activateTab, tabs } from "../useTabs";
import { askConfirm } from "./confirm";
import { activeSessionTab, allSessionTabs, markSessionTabStopped } from "./sessionState";
import { store } from "./store";
import { interrupt } from "./turnControl";
import type { SessionTab } from "./types";


/**
 * ComposerBar 注册的“添加为会话附件”处理器：tabId → handler。
 * 取代旧的单例 window 钩子：多个会话标签同时挂载时，只有注册表能保证
 * 附件路由到当前活动的会话输入区，而不是最后挂载的那个。
 */
const composerAddHandlers = new Map<string, (a: UserInput) => void>();


/** ComposerBar 挂载/创建编辑器后注册本会话标签的附件处理器 */
export function registerComposerAddHandler(
  tabId: string,
  fn: (a: UserInput) => void,
) {
  composerAddHandlers.set(tabId, fn);
}


/** ComposerBar 卸载时注销，避免路由到已关闭会话 */
export function unregisterComposerAddHandler(tabId: string) {
  composerAddHandlers.delete(tabId);
}


/**
 * 资源面板“添加为会话附件”路由：按当前活动会话标签 id 查找其 ComposerBar
 * 处理器并调用；无活动会话或处理器缺失时返回 false，由调用方兜底。
 */
export function addAttachmentToActiveSession(a: UserInput): boolean {
  const tab = activeSessionTab();
  if (!tab) return false;
  const fn = composerAddHandlers.get(tab.id);
  if (!fn) return false;
  fn(a);
  return true;
}


/**
 * 切换到指定会话标签：统一列表内按 id 激活（live 字段由投影 watch 同步）。
 * 会话多开：切换不确认、不中断后台回合。
 */
export async function switchSessionTab(id: string): Promise<boolean> {
  const target = tabs.find(
    (t): t is SessionTab => t.kind === TabKind.Chat && t.id === id,
  );
  if (!target) return false;
  if (activeTabId.value === id) return true;
  activateTab(id);
  return true;
}


/**
 * 关闭会话标签：运行中的会话先确认并中断（多开时仅关闭才停止）；关闭活动标签时
 * 自动切到相邻会话标签，无剩余会话时复位 live 默认态并把视图切到剩余标签（或空）。
 */
export async function closeSessionTab(id: string): Promise<void> {
  const sessions = allSessionTabs();
  const sidx = sessions.findIndex((t) => t.id === id);
  if (sidx < 0) return;
  const tab = sessions[sidx];
  const idx = tabs.indexOf(tab);
  if (tab.threadId && (tab.turnActive || tab.goalText)) {
    const ok = await askConfirm({
      title: "关闭会话标签",
      message: "该会话仍在进行中，关闭将停止当前回合。是否继续？",
      confirmLabel: "停止并关闭",
      cancelLabel: "取消",
    });
    if (!ok) return;
    void interrupt(tab.threadId, tab.currentTurnId);
    markSessionTabStopped(tab);
  }
  const wasActive = activeTabId.value === id;
  tabs.splice(idx, 1);
  if (wasActive) {
    // 相邻会话标签（sessions 为移除前快照：同位置或前一个）；无会话时视图切到剩余标签
    const next = sessions[sidx + 1] ?? sessions[sidx - 1] ?? null;
    if (next) activateTab(next.id);
    else if (tabs.length > 0) activateTab(tabs[Math.min(idx, tabs.length - 1)]!.id);
    else activeTabId.value = "";
  }
  // 释放该线程的本地消息缓存（无其它标签引用时）
  if (
    tab.threadId &&
    !allSessionTabs().some((t) => t.threadId === tab.threadId)
  ) {
    delete store.itemsByThread[tab.threadId];
    delete store.activeWorkByThread[tab.threadId];
  }
}


/**
 * 关闭全部会话标签（“关闭所有标签”用）：运行中的跳过并计数；
 * 允许关闭到 0 个会话标签（主区域显示空状态）。
 * 返回跳过的运行中标签数量。
 */
export async function closeAllSessionTabs(): Promise<number> {
  let skipped = 0;
  for (const tab of allSessionTabs()) {
    if (tab.turnActive || tab.goalText) {
      skipped++;
      continue;
    }
    await closeSessionTab(tab.id);
  }
  return skipped;
}
