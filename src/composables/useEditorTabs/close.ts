import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { askConfirm, closeSessionTab } from "../useCodex";
import { releaseTerminal } from "../useTerminalEvents";
import { TabKind } from "../../lib/tabs";
import { activeTabId, tabs } from "../useTabs";
import type { Tab } from "../useTabs";
import { isTerminalBusy, saveFileTab } from "./open";
import type { EditorTab, FileEditorTab } from "./types";

/** 待关闭的脏文件标签 id（由编辑面板弹确认层） */
export const pendingCloseId = ref<string | null>(null);

/** 有未保存更改的文件型标签（文本文件） */
function isDirtyEditableTab(tab: EditorTab): boolean {
  return tab.kind === TabKind.File && tab.dirty;
}

/** 释放标签后端资源：终端进程结束（幂等，失败静默） */
function disposeTab(tab: EditorTab): void {
  if (tab.kind === TabKind.Terminal) {
    void invoke("terminal_kill", { id: tab.id }).catch(() => {});
  }
}

/**
 * 关闭标签：脏文件先挂起确认（pendingCloseId），确认后由 saveTabAndClose/
 * discardTabAndClose 完成；运行中的终端先弹全局确认，确认后终止进程并移除，
 * 取消则保留；空闲/已退出/启动失败的终端直接结束进程并移除。
 */
export async function closeTab(id: string): Promise<void> {
  const tab = tabs.find((t) => t.id === id);
  if (!tab) return;
  if (tab.kind === TabKind.Session) {
    await closeSessionTab(tab.id);
    return;
  }
  if (isDirtyEditableTab(tab)) {
    pendingCloseId.value = id;
    return;
  }
  if (isTerminalBusy(tab)) {
    const ok = await askConfirm({
      title: "关闭终端",
      message: "终端正在执行命令，关闭将终止该进程。是否继续？",
      confirmLabel: "终止并关闭",
      cancelLabel: "取消",
    });
    if (!ok) return;
  }
  disposeTab(tab);
  removeTab(id);
}

/**
 * 统一关闭入口（关闭按钮/中键/批量关闭共用）：统一列表内按 id 关闭——
 * 会话标签走 closeSessionTab（运行中确认并中断），编辑器标签走 closeTab
 * （脏文件挂起、运行中终端确认并终止）。
 */
export async function closeAnyTab(tab: Tab): Promise<void> {
  await closeTab(tab.id);
}

export function cancelClose(): void {
  pendingCloseId.value = null;
}

export async function saveTabAndClose(id: string): Promise<void> {
  pendingCloseId.value = null;
  const ok = await saveFileTab(id);
  if (ok) removeTab(id);
}

export function discardTabAndClose(id: string): void {
  pendingCloseId.value = null;
  removeTab(id);
}

/**
 * 按统一列表顺序关闭 [start, end) 区间内的标签（含会话标签）：运行中的会话
 * 跳过计数（不逐个确认），未保存文件/运行中终端跳过计数；返回跳过数量。
 */
async function closeTabRange(start: number, end: number): Promise<number> {
  let skipped = 0;
  const snapshot = [...tabs].slice(start, end);
  for (const tab of snapshot) {
    if (tab.kind === TabKind.Session) {
      if (tab.turnActive || tab.goalText) {
        skipped++;
        continue;
      }
      await closeSessionTab(tab.id);
      continue;
    }
    const t = tab as EditorTab;
    if (isDirtyEditableTab(t)) {
      skipped++;
      continue;
    }
    if (isTerminalBusy(t)) {
      skipped++;
      continue;
    }
    disposeTab(t);
    removeTab(t.id);
  }
  return skipped;
}

/** 关闭目标标签左侧所有标签（含会话标签，不含目标本身）；返回跳过数量 */
export async function closeTabsToLeftAll(id: string): Promise<number> {
  const idx = tabs.findIndex((t) => t.id === id);
  if (idx < 0) return 0;
  return closeTabRange(0, idx);
}

/** 关闭目标标签右侧所有标签（含会话标签，不含目标本身）；返回跳过数量 */
export async function closeTabsToRightAll(id: string): Promise<number> {
  const idx = tabs.findIndex((t) => t.id === id);
  if (idx < 0) return 0;
  return closeTabRange(idx + 1, tabs.length);
}

/** 关闭全部标签（会话+编辑器）；返回跳过数量 */
export async function closeAllTabs(): Promise<number> {
  return closeTabRange(0, tabs.length);
}

function removeTab(id: string): void {
  const idx = tabs.findIndex((t) => t.id === id);
  if (idx < 0) return;
  const kind = tabs[idx].kind;
  const wasActive = activeTabId.value === id;
  tabs.splice(idx, 1);
  if (kind === TabKind.Terminal) releaseTerminal(id);
  if (kind === TabKind.Session) return; // 会话标签的激活切换由 useCodex 的会话关闭流程处理
  if (wasActive) {
    const next = tabs[Math.max(0, idx - 1)] ?? tabs[0];
    activeTabId.value = next ? next.id : "";
  }
}

/** 有未保存更改的文件型标签（文本文件，供关闭应用守卫使用） */
export function dirtyEditableTabs(): FileEditorTab[] {
  return tabs.filter(
    (t): t is FileEditorTab => t.kind === TabKind.File && t.dirty,
  );
}

/** 保存全部脏标签；全部成功返回 true（任一失败则不关闭应用） */
export async function saveAllDirtyTabs(): Promise<boolean> {
  const results = await Promise.all(
    dirtyEditableTabs().map((t) => saveFileTab(t.id)),
  );
  return results.every(Boolean);
}

/** 测试专用：清空文件/diff/预览/终端标签（保留会话标签） */
export function __resetEditorTabsForTest(): void {
  const removed = tabs.filter((t) => t.kind !== TabKind.Session);
  for (const t of removed) tabs.splice(tabs.indexOf(t), 1);
  if (!tabs.some((t) => t.id === activeTabId.value)) {
    activeTabId.value = tabs[0]?.id ?? "";
  }
  pendingCloseId.value = null;
}
