// useCodex 拆分模块：会话标签底层状态（原 useCodex.ts 的一部分，纯移动，行为不变）
import { reactive, watch } from "vue";
import { pathBaseName } from "../../lib/format";
import { TabIcon, TabKind } from "../../lib/tabs";
import { activeTab, activeTabId, activateTab, tabs } from "../useTabs";
// 主窗口标题跟随活动 Tab 的模块级 watch（在此导入以确保在 useCodex 各模块图中均被接线）
import "./windowTitle";
import { resolveSessionWorkspace } from "./items";
import { threadTitle } from "./selectors";
import { store } from "./store";
import type { SessionTab } from "./types";


let sessionTabSeq = 0;


/** 会话标签唯一 id：线程绑定前/后均稳定（多标签下编辑器标签 key 不变） */
function nextSessionTabId(): string {
  return `session-${Date.now()}-${++sessionTabSeq}`;
}


/** 新对话（未绑定线程）标签的默认状态 */
export function freshSessionTab(): SessionTab {
  return reactive({
    id: nextSessionTabId(),
    kind: TabKind.Chat,
    title: "新建会话",
    icon: TabIcon.Chat,
    threadId: null,
    name: "",
    nameIsFirstMessage: false,
    permissionMode: store.settings.default_permission,
    taskMode: "default",
    model: null,
    effort: null,
    plugins: { plugins: [], loaded: false },
    skills: { skills: [], loaded: false },
    draftJson: JSON.stringify({ type: "doc", content: [] }),
    draftAttachments: [],
    draftRefs: {},
    origin: null,
    workspace: null,
    resumedThreadId: null,
    turnActive: false,
    currentTurnId: null,
    turnInterrupted: false,
    goalText: null,
    goalStatus: null,
    goalArmed: false,
    threadTokenUsage: null,
    followupQueue: [],
    attachments: [],
    planPrompt: null,
    plan: null,
    loading: false,
    creatingChat: false,
    newChatWorkspace: null,
    interactions: [],
  } as SessionTab);
}


/** 按线程查找已打开的会话标签（唯一性约束：至多一个） */
export function findSessionTabByThread(
  threadId: string | null | undefined,
): SessionTab | undefined {
  if (!threadId) return undefined;
  return tabs.find(
    (t): t is SessionTab => t.kind === TabKind.Chat && t.threadId === threadId,
  );
}


/** 该线程是否已作为会话标签打开 */
export function isThreadOpen(threadId: string): boolean {
  return !!findSessionTabByThread(threadId);
}


/** 该线程的会话标签是否正在后台/前台运行回合 */
export function isThreadRunning(threadId: string): boolean {
  return !!findSessionTabByThread(threadId)?.turnActive;
}


/** live 字段当前投影的会话标签 id（活动标签为会话时随切换更新；文件/终端活动时保持最近会话） */
let activeSessionTabId: string | null = null;


/** 当前激活的会话标签（无则 null）：优先当前显示标签，否则最近投影的会话 */
export function activeSessionTab(): SessionTab | null {
  const t = activeTab.value;
  if (t && t.kind === TabKind.Chat) return t;
  if (!activeSessionTabId) return null;
  const s = tabs.find((x) => x.id === activeSessionTabId);
  return s && s.kind === TabKind.Chat ? s : null;
}


/** 统一列表中的会话标签集合（事件路由/批量处理用） */
export function allSessionTabs(): SessionTab[] {
  return tabs.filter((t): t is SessionTab => t.kind === TabKind.Chat);
}


/**
 * 会话标签显示标题：恒为 `{sessionroot} / {标题内容}` 格式。
 * sessionroot 取标签解析后工作目录的目录名（线程 cwd → 新对话预选目录 → workspace 兜底）；
 * 标题内容取名称/摘要，无线程的新对话兜底“新建会话”；无任何可用目录时降级为仅标题内容。
 */
export function sessionTabTitle(tab: SessionTab): string {
  const root = resolveSessionWorkspace(tab);
  const folder = root ? pathBaseName(root) : "";
  const summary = tab.threadId
    ? store.threads.find((t) => t.id === tab.threadId)
    : undefined;
  const title = tab.name || (summary ? threadTitle(summary) : "新建会话");
  return folder ? `${folder} / ${title}` : title;
}


/** 乐观复位被停止/关闭的标签回合状态（interrupt 异步完成前先复位展示） */
export function markSessionTabStopped(tab: SessionTab | null | undefined) {
  if (!tab) return;
  tab.turnActive = false;
  tab.currentTurnId = null;
  tab.turnInterrupted = true;
  tab.goalText = null;
  tab.goalStatus = null;
  tab.goalArmed = false;
}


/**
 * 当前显示标签为会话时维护 activeSessionTabId（最近会话回退）；
 * 无任何标签时清空。会话状态已全部落在标签对象上，无需投影。
 */
function syncActiveSessionProjection() {
  const t = activeTab.value;
  if (t && t.kind === TabKind.Chat) {
    activeSessionTabId = t.id;
    return;
  }
  if (!t && activeSessionTabId !== null) {
    activeSessionTabId = null;
  }
}


// 活动标签变化（切换/关闭）时同步 live 字段投影；flush sync 保证切换后立即可用
watch(activeTab, () => syncActiveSessionProjection(), {
  immediate: true,
  flush: "sync",
});


/**
 * 无条件移除会话标签（线程不存在等异常路径）：处理活动切换、live 投影与缓存清理。
 */
export function dropSessionTab(tab: SessionTab) {
  const idx = tabs.indexOf(tab);
  if (idx < 0) return;
  const sessions = allSessionTabs();
  const sidx = sessions.indexOf(tab);
  const wasActive = activeTabId.value === tab.id;
  tabs.splice(idx, 1);
  if (wasActive) {
    const next = sessions[sidx + 1] ?? sessions[sidx - 1] ?? null;
    if (next) activateTab(next.id);
    else if (tabs.length > 0) activateTab(tabs[Math.min(idx, tabs.length - 1)]!.id);
    else activeTabId.value = "";
  }
  if (
    tab.threadId &&
    !allSessionTabs().some((t) => t.threadId === tab.threadId)
  ) {
    delete store.itemsByThread[tab.threadId];
    delete store.activeWorkByThread[tab.threadId];
  }
}


/** 仅测试用：清空会话标签状态 */
export function __resetSessionTabsForTest() {
  for (const tab of allSessionTabs()) {
    tabs.splice(tabs.indexOf(tab), 1);
  }
  activeSessionTabId = null;
  if (!tabs.some((t) => t.id === activeTabId.value)) {
    activeTabId.value = tabs[0]?.id ?? "";
  }
}
