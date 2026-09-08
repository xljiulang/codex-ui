// useCodex 拆分模块：会话标签底层状态（原 useCodex.ts 的一部分，纯移动，行为不变）
import { reactive, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { TabIcon, TabKind } from "../../lib/tabs";
import { activeTab, activeTabId, activateTab, tabs } from "../useTabs";
import type { SessionStateInfo } from "../../lib/types";
// 主窗口标题跟随活动 Tab 的模块级 watch（在此导入以确保在 useCodex 各模块图中均被接线）
import "./windowTitle";
import { threadTitle } from "./selectors";
import { store } from "./store";
import { setToast } from "./toast";
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
    kind: TabKind.Session,
    title: "新建会话",
    icon: TabIcon.Session,
    threadId: null,
    name: "",
    nameIsFirstMessage: false,
    permissionMode: store.settings.default_permission,
    collaborationMode: "default",
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
    creatingSession: false,
    newSessionWorkspace: null,
    interactions: [],
  } as SessionTab);
}


/** 按线程查找已打开的会话标签（唯一性约束：至多一个） */
export function findSessionTabByThread(
  threadId: string | null | undefined,
): SessionTab | undefined {
  if (!threadId) return undefined;
  return tabs.find(
    (t): t is SessionTab => t.kind === TabKind.Session && t.threadId === threadId,
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
  if (t && t.kind === TabKind.Session) return t;
  if (!activeSessionTabId) return null;
  const s = tabs.find((x) => x.id === activeSessionTabId);
  return s && s.kind === TabKind.Session ? s : null;
}


/** 统一列表中的会话标签集合（事件路由/批量处理用） */
export function allSessionTabs(): SessionTab[] {
  return tabs.filter((t): t is SessionTab => t.kind === TabKind.Session);
}


/**
 * 会话标签显示标题：仅取标题内容（名称/摘要，无线程的新对话兜底“新建会话”）。
 * 不再拼接工作目录名前缀（会话标签与窗口标题均使用本标题）。
 */
export function sessionTabTitle(tab: SessionTab): string {
  const summary = tab.threadId
    ? store.threads.find((t) => t.id === tab.threadId)
    : undefined;
  return tab.name || (summary ? threadTitle(summary) : "新建会话");
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
function trackActiveSessionTabId() {
  const t = activeTab.value;
  if (t && t.kind === TabKind.Session) {
    activeSessionTabId = t.id;
    return;
  }
  if (!t && activeSessionTabId !== null) {
    activeSessionTabId = null;
  }
}


// 活动标签变化（切换/关闭）时更新最近会话标签 id；flush sync 保证切换后立即可用
watch(activeTab, () => trackActiveSessionTabId(), {
  immediate: true,
  flush: "sync",
});


/**
 * 无条件移除会话标签（线程不存在等异常路径）：处理活动切换、最近会话回退与缓存清理。
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

/**
 * 从统一会话状态（sessions.json）回填某会话标签的权限/模型/推理强度；
 * 无记录或 threadId 为空时保持默认。仅在历史会话打开 / 恢复时调用。
 * 模型/推理强度若已下架（不在当前模型列表），自动回退默认并写回 + toast 提示。
 */
export async function hydrateSessionState(tab: SessionTab): Promise<void> {
  if (!tab.threadId) return;
  try {
    const s = await invoke<SessionStateInfo | null>("sessions_get", {
      threadId: tab.threadId,
    });
    if (!s) return;
    if (s.permissionMode) tab.permissionMode = s.permissionMode;
    let model = s.model ?? null;
    let effort = s.effort ?? null;
    let note = "";
    // 模型列表已加载才做可用性清洗（未加载时按原值应用，避免误判）
    if (store.modelsLoaded && store.models.length) {
      if (model) {
        const known = store.models.find((m) => m.model === model);
        if (!known) {
          const fallback =
            store.models.find((m) => m.isDefault) ?? store.models[0];
          if (fallback) {
            const orig = model;
            model = fallback.model;
            effort = fallback.defaultReasoningEffort || null;
            note = `会话模型「${orig}」已不可用，已回退到「${fallback.displayName || fallback.model}」`;
          }
        } else if (effort) {
          // 档位下架：受支持列表非空且不含当前档 → 恢复默认强度
          const supported = known.supportedReasoningEfforts;
          if (
            supported.length &&
            !supported.some((e) => e.reasoningEffort === effort)
          ) {
            const orig = effort;
            effort = null;
            note = `推理强度「${orig}」已不可用，已恢复默认`;
          }
        }
      } else if (effort) {
        // model 为默认：按默认模型校验强度
        const def = store.models.find((m) => m.isDefault) ?? store.models[0];
        if (
          def &&
          def.supportedReasoningEfforts.length &&
          !def.supportedReasoningEfforts.some(
            (e) => e.reasoningEffort === effort,
          )
        ) {
          const orig = effort;
          effort = null;
          note = `推理强度「${orig}」已不可用，已恢复默认`;
        }
      }
    }
    tab.model = model;
    tab.effort = effort;
    if (note) {
      void saveSessionState(tab); // 落盘回退后的值，避免每次打开重复回退
      setToast(note);
    }
  } catch {
    // 读取失败保持默认，不阻断会话打开
  }
}

/**
 * 把某会话标签的权限/模型/推理强度写入统一会话状态（保留微信绑定）。
 * 可选 `resolved` 覆盖 model/effort（新建会话用实际生效值固化默认），不传则读 tab.*。
 */
export async function saveSessionState(
  tab: SessionTab,
  resolved?: { model?: string | null; effort?: string | null },
): Promise<void> {
  if (!tab.threadId) return;
  try {
    await invoke("sessions_update", {
      threadId: tab.threadId,
      permissionMode: tab.permissionMode,
      model: resolved ? resolved.model ?? null : tab.model,
      effort: resolved ? resolved.effort ?? null : tab.effort,
    });
  } catch {
    // 写失败不阻断 UI（下次变更或会话打开会再尝试）
  }
}

/**
 * 用 thread/resume 返回的已解析模型/强度回填标签并落盘：服务端为唯一事实源
 * （含持久化的线程模型），避免本地 sessions.json 旧值/缺失时显示与实际不一致。
 * 仅当字段存在时覆盖；返回空 model 视为无效不动。
 */
export function applyResumedSettings(tab: SessionTab, res: unknown): void {
  if (!res || typeof res !== "object") return;
  const r = res as { model?: unknown; reasoningEffort?: unknown };
  const model = typeof r.model === "string" ? r.model.trim() : "";
  let changed = false;
  if (model && model !== tab.model) {
    tab.model = model;
    changed = true;
  }
  if ("reasoningEffort" in r) {
    const effort = typeof r.reasoningEffort === "string" ? r.reasoningEffort : null;
    if (effort !== tab.effort) {
      tab.effort = effort;
      changed = true;
    }
  }
  if (changed) void saveSessionState(tab);
}

/** 删除会话时清空其统一状态记录。 */
export async function removeSessionState(threadId: string): Promise<void> {
  try {
    await invoke("sessions_remove", { threadId });
  } catch {
    // 清空失败静默（记录残留无害）
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
