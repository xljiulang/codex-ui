// useCodex 拆分模块：会话标签底层状态（原 useCodex.ts 的一部分，纯移动，行为不变）
import { reactive, watch } from "vue";
import { pathBaseName } from "../../lib/format";
import { TabIcon, TabKind } from "../../lib/tabs";
import { activeTab, activeTabId, activateTab, tabs } from "../useTabs";
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
    taskMode: "execute",
    model: store.model,
    effort: store.effort,
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
    loading: false,
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


/** 把标签记录恢复到 live 字段（切换/关闭标签时用） */
function restoreSession(tab: SessionTab) {
  store.currentThreadId = tab.threadId;
  store.currentThreadName = tab.name;
  store.permissionMode = tab.permissionMode;
  store.taskMode = tab.taskMode;
  store.model = tab.model;
  store.effort = tab.effort;
  store.currentThreadOrigin = tab.origin;
  store.currentThreadWorkspace = tab.workspace;
  store.resumedThreadId = tab.resumedThreadId;
  store.turnActive = tab.turnActive;
  store.currentTurnId = tab.currentTurnId;
  store.turnInterrupted = tab.turnInterrupted;
  store.goalText = tab.goalText;
  store.goalStatus = tab.goalStatus;
  store.goalArmed = tab.goalArmed;
  store.threadTokenUsage = tab.threadTokenUsage;
  store.planPrompt = tab.planPrompt;
  store.loading = tab.loading;
  store.newChatWorkspace = tab.newChatWorkspace;
  store.followupQueue = [...tab.followupQueue];
  store.attachments = [...tab.attachments];
}


/** 把 live 字段快照到活动标签记录（活动标签的字段变化后由 watch 持续调用） */
export function syncActiveSessionTab() {
  const tab = activeSessionTab();
  if (!tab) return;
  tab.threadId = store.currentThreadId;
  tab.name = store.currentThreadName;
  tab.permissionMode = store.permissionMode;
  tab.taskMode = store.taskMode;
  tab.model = store.model;
  tab.effort = store.effort;
  tab.origin = store.currentThreadOrigin;
  tab.workspace = store.currentThreadWorkspace;
  tab.resumedThreadId = store.resumedThreadId;
  tab.turnActive = store.turnActive;
  tab.currentTurnId = store.currentTurnId;
  tab.turnInterrupted = store.turnInterrupted;
  tab.goalText = store.goalText;
  tab.goalStatus = store.goalStatus;
  tab.goalArmed = store.goalArmed;
  tab.threadTokenUsage = store.threadTokenUsage;
  tab.planPrompt = store.planPrompt ? { ...store.planPrompt } : null;
  tab.loading = store.loading;
  tab.newChatWorkspace = store.newChatWorkspace;
  tab.followupQueue = [...store.followupQueue];
  tab.attachments = [...store.attachments];
  tab.title = sessionTabTitle(tab);
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


/** 复位 live 字段到“无会话标签”默认态（不创建标签；允许 0 个会话标签） */
function resetLiveSessionState() {
  restoreSession(freshSessionTab());
}


/**
 * 当前显示标签为会话时：同步 activeSessionTabId 并投影 live 字段；
 * 无任何标签时复位 live 默认态。切换/关闭/复位统一走这里。
 */
function syncActiveSessionProjection() {
  const t = activeTab.value;
  if (t && t.kind === TabKind.Chat) {
    if (activeSessionTabId !== t.id) {
      activeSessionTabId = t.id;
      restoreSession(t);
    }
    return;
  }
  if (!t && activeSessionTabId !== null) {
    activeSessionTabId = null;
    resetLiveSessionState();
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
