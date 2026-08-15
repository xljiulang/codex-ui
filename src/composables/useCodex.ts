import { computed, nextTick, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, ProgressBarStatus } from "@tauri-apps/api/window";

import type {
  AppSettings,
  PendingInteraction,
  ServerStatus,
  ThreadItem,
  ThreadSummary,
  Turn,
  UserInput,
  PermissionId,
} from "../lib/types";
import {
  PERMISSION_MODES,
  permissionMode,
  toApprovalPolicy,
  toApprovalsReviewer,
  toSandbox,
  toSandboxPolicy,
} from "../lib/permissions";
import {
  buildTurnInput,
  stripMentionContext,
} from "../lib/mention";
import { playNotificationSound } from "../lib/sound";
import { pathBaseName } from "../lib/format";
import { focusComposer } from "../lib/composerFocus";
import {
  friendlyServerError,
  friendlyServerMessage,
} from "../lib/serverMessages";
import { applyTheme } from "./useTheme";
import { TabIcon, TabKind, type EditorTabBase } from "../lib/tabs";

const defaultSettings = (): AppSettings => ({
  codex_path: null,
  sound_enabled: true,
  enter_to_send: true,
  followup_mode: "adjust",
  theme: "blue",
  default_permission: "ask-for-approval",
  memory_mode: "disabled",
});

interface ModelInfo {
  id: string;
  model: string;
  displayName: string;
  description: string;
  hidden: boolean;
  isDefault: boolean;
  supportedReasoningEfforts: { reasoningEffort: string; description: string }[];
  defaultReasoningEffort: string;
}

/** 线程目标的持久状态（thread/goal/get 与 thread/goal/updated 携带；值对齐协议绑定 ThreadGoalStatus） */
export type GoalStatus =
  | "active"
  | "paused"
  | "blocked"
  | "usageLimited"
  | "budgetLimited"
  | "complete";

export function isGoalStatus(v: unknown): v is GoalStatus {
  return (
    v === "active" ||
    v === "paused" ||
    v === "blocked" ||
    v === "usageLimited" ||
    v === "budgetLimited" ||
    v === "complete"
  );
}

/** 服务端终态：目标已完成/预算耗尽/用量受限/阻塞/暂停，客户端收到后自动清目标并复位 flag */
const GOAL_TERMINAL_STATUSES = new Set<GoalStatus>([
  "complete",
  "budgetLimited",
  "usageLimited",
  "blocked",
  "paused",
]);

function isGoalTerminalStatus(v: unknown): v is GoalStatus {
  return typeof v === "string" && GOAL_TERMINAL_STATUSES.has(v as GoalStatus);
}

/** 终态 toast 文案：目标已完成/预算耗尽/用量受限/已阻塞/已暂停 */
function goalStatusToast(status: GoalStatus): string {
  switch (status) {
    case "complete":
      return "目标已完成";
    case "budgetLimited":
      return "目标预算耗尽";
    case "usageLimited":
      return "目标用量受限";
    case "blocked":
      return "目标已阻塞";
    case "paused":
      return "目标已暂停";
    default:
      return "";
  }
}

/** 计划模式回合完成后的“计划已就绪”确认弹窗数据（纯前端 UX，非协议交互） */
export interface PlanPrompt {
  threadId: string;
  turnId: string;
  planText: string;
}

/**
 * 会话标签：左侧标签区的每个“会话”标签对应一个打开的会话
 * （threadId 非空 = 已绑定线程；null = 待发送首条消息的新对话）。
 * 活动标签的实时状态以 store 的 current* 字段为准（由自动同步 watch 落回本记录）；
 * 切换标签时以本记录恢复 live 字段。消息列表本身按线程存于 itemsByThread，无需复制。
 */
export interface SessionTab extends EditorTabBase {
  kind: (typeof TabKind)["Chat"];
  title: string;
  icon: (typeof TabIcon)["Chat"];
  /** 标签唯一 id（线程绑定前后保持稳定，供编辑器标签 key 使用） */
  id: string;
  /** 绑定的线程 id；一个会话最多对应一个标签（唯一性约束） */
  threadId: string | null;
  /** 会话名称（自动生成/手动重命名，随 thread/name/updated 同步） */
  name: string;
  /** 会话名是否来自「首条消息内容」（供 AI 总结覆盖；手动重命名后置 false） */
  nameIsFirstMessage: boolean;
  /** 会话私有：权限模式（新会话取默认权限） */
  permissionMode: PermissionId;
  /** 会话私有：任务模式 */
  taskMode: "execute" | "plan";
  /** 会话私有：模型（null = 服务端默认） */
  model: string | null;
  /** 会话私有：推理强度（null = 默认） */
  effort: string | null;
  /** 会话私有：输入框草稿（Tiptap 文档 JSON 序列化，ComposerBar 维护） */
  draftJson: string;
  /** 会话私有：输入框附件区（ComposerBar 维护） */
  draftAttachments: UserInput[];
  /** 会话私有：输入框内联引用 map（refId → 附件，ComposerBar 维护） */
  draftRefs: Record<string, UserInput>;
  origin: "new" | "history" | null;
  workspace: string | null;
  resumedThreadId: string | null;
  turnActive: boolean;
  currentTurnId: string | null;
  turnInterrupted: boolean;
  goalText: string | null;
  goalStatus: GoalStatus | null;
  goalArmed: boolean;
  threadTokenUsage: { used: number; window: number | null } | null;
  followupQueue: { text: string; attachments: UserInput[] }[];
  attachments: UserInput[];
  planPrompt: PlanPrompt | null;
  loading: boolean;
  /** 新建对话时可选的项目目录（null = 使用启动工作目录） */
  newChatWorkspace: string | null;
  /** 该标签待处理的交互（审批/提问/elicitation），按 threadId 路由 */
  interactions: PendingInteraction[];
}

/** @ 菜单中展示的插件条目（plugin/list 归一化结果） */
export interface PluginItem {
  id: string;
  name: string;
  displayName: string;
  description: string;
  path: string;
  iconPath: string;
  iconUrl: string;
  brandColor: string;
}

/** $ 菜单与回显悬浮提示共用的技能条目（skills/list 归一化结果） */
export interface SkillItem {
  name: string;
  key: string;
  path: string;
  /** 长描述（悬浮提示用） */
  desc: string;
  /** 短描述（$ 菜单展示用，优先 interface.shortDescription） */
  shortDesc: string;
}

/** 未创建会话（新对话编辑态）的插件缓存 key */
export const NEW_CHAT_PLUGIN_KEY = "__new__";

/** 右侧面板 Tab：资源管理器 / 会话历史 / Git */
export type PanelTab = "history" | "resources" | "git";

export const store = reactive({
  server: {
    connected: false,
    startupWorkspace: "",
    codexPath: null as string | null,
    logs: [] as string[],
  },
  threads: [] as ThreadSummary[],
  searchActive: false,
  searchSnippets: {} as Record<string, string>,
  currentThreadId: null as string | null,
  currentThreadName: "",
  currentThreadOrigin: null as "new" | "history" | null,
  currentThreadWorkspace: null as string | null,
  resumedThreadId: null as string | null,
  /** 已打开的会话标签列表（多会话标签） */
  sessionTabs: [] as SessionTab[],
  /** 当前激活的会话标签 id（null = 尚未初始化任何标签） */
  activeSessionId: null as string | null,
  /** 活动标签工作区覆盖（文件/diff/预览/终端标签由 EditorPane 写入；null=跟随会话工作区） */
  workspace: null as string | null,
  itemsByThread: {} as Record<string, ThreadItem[]>,
  // 每个线程“进行中工作”计数（流式文本/进行中工具），避免渲染时全量扫描
  activeWorkByThread: {} as Record<string, number>,
  // 任意消息变更（新增/完成/流式增量）都会递增，供吸底滚动等做 O(1) 变更感知
  itemsRev: 0,
  // 用户手动发送计数器：每次 ComposerBar 提交（sendPrompt）递增，
  // 供 ChatView 在发送后强制恢复吸底（排队消息自动发送不递增）
  userSendRev: 0,
  turnActive: false,
  turnInterrupted: false,
  currentTurnId: null as string | null,
  followupQueue: [] as { text: string; attachments: UserInput[] }[],
  interactions: [] as PendingInteraction[],
  settings: defaultSettings(),
  // 启动加载态：init() 完成（含超时兜底）前为 true，App 据此显示加载动画
  booting: true,
  loadingHistory: false,
  loading: false,
  busy: false,
  currentModel: "",
  // 进程级设置：权限模式 / 模型 / 推理强度，仅当前运行期有效，不写入配置文件
  permissionMode: "ask-for-approval" as PermissionId,
  model: null as string | null,
  effort: null as string | null,
  models: [] as ModelInfo[],
  modelsLoaded: false,
  // 对话级插件缓存：key 为 currentThreadId（未创建会话时为 NEW_CHAT_PLUGIN_KEY）
  threadPlugins: {} as Record<
    string,
    { plugins: PluginItem[]; loaded: boolean }
  >,
  skills: [] as SkillItem[],
  skillsLoaded: false,
  threadTokenUsage: null as { used: number; window: number | null } | null,
  // 新建对话时可选的项目目录（null = 使用启动工作目录）
  newChatWorkspace: null as string | null,
  taskMode: "execute" as "execute" | "plan",
  goalText: null as string | null,
  /** 当前线程目标状态（thread/goal 事件同步，null = 未挂载目标） */
  goalStatus: null as GoalStatus | null,
  /** 目标 flag 勾选态：勾选后无目标值，首条消息纯文本即目标（纯客户端状态，不跨会话） */
  goalArmed: false,
  attachments: [] as UserInput[],
  showSettings: false,
  /** 右侧面板当前激活 Tab：会话/资源/Git，默认会话（首个 Tab） */
  panelTab: "history" as PanelTab,
  permOpen: false,
  taskOpen: false,
  modelOpen: false,
  toast: "",
  /** 全局确认弹窗（会话切换等需用户选择） */
  confirm: null as (ConfirmRequest & { resolve: (ok: boolean) => void }) | null,
  /** 计划模式回合完成后待用户确认的“计划已就绪”弹窗 */
  planPrompt: null as PlanPrompt | null,
});

let sessionTabSeq = 0;

/** 会话标签唯一 id：线程绑定前/后均稳定（多标签下编辑器标签 key 不变） */
function nextSessionTabId(): string {
  return `session-${Date.now()}-${++sessionTabSeq}`;
}

/** 新对话（未绑定线程）标签的默认状态 */
function freshSessionTab(): SessionTab {
  return {
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
  };
}

/** 按线程查找已打开的会话标签（唯一性约束：至多一个） */
export function findSessionTabByThread(
  threadId: string | null | undefined,
): SessionTab | undefined {
  if (!threadId) return undefined;
  return store.sessionTabs.find((t) => t.threadId === threadId);
}

/** 该线程是否已作为会话标签打开 */
export function isThreadOpen(threadId: string): boolean {
  return !!findSessionTabByThread(threadId);
}

/** 该线程的会话标签是否正在后台/前台运行回合 */
export function isThreadRunning(threadId: string): boolean {
  return !!findSessionTabByThread(threadId)?.turnActive;
}

/** 当前激活的会话标签（无则 null） */
export function activeSessionTab(): SessionTab | null {
  return store.sessionTabs.find((t) => t.id === store.activeSessionId) ?? null;
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
function syncActiveSessionTab() {
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
function markSessionTabStopped(tab: SessionTab | null | undefined) {
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
  store.activeSessionId = null;
  restoreSession(freshSessionTab());
}

/**
 * 切换到指定会话标签：快照当前标签 → 恢复目标标签 → 更新 live 字段。
 * 会话多开：切换不确认、不中断后台回合。
 */
export async function switchSessionTab(id: string): Promise<boolean> {
  const target = store.sessionTabs.find((t) => t.id === id);
  if (!target) return false;
  if (id === store.activeSessionId) return true;
  syncActiveSessionTab();
  store.activeSessionId = id;
  restoreSession(target);
  return true;
}

/**
 * 关闭会话标签：运行中的会话先确认并中断（多开时仅关闭才停止）；关闭活动标签时
 * 自动切到相邻标签，无剩余标签时新建一个空标签兜底。
 */
export async function closeSessionTab(id: string): Promise<void> {
  const idx = store.sessionTabs.findIndex((t) => t.id === id);
  if (idx < 0) return;
  const tab = store.sessionTabs[idx];
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
  const wasActive = store.activeSessionId === id;
  store.sessionTabs.splice(idx, 1);
  if (wasActive) {
    const next =
      store.sessionTabs[Math.min(idx, store.sessionTabs.length - 1)] ?? null;
    if (next) {
      store.activeSessionId = next.id;
      restoreSession(next);
    } else {
      // 允许 0 个会话标签：复位 live 字段到无会话默认态
      resetLiveSessionState();
    }
  }
  // 释放该线程的本地消息缓存（无其它标签引用时）
  if (
    tab.threadId &&
    !store.sessionTabs.some((t) => t.threadId === tab.threadId)
  ) {
    delete store.itemsByThread[tab.threadId];
    delete store.activeWorkByThread[tab.threadId];
  }
}

/** 关闭除当前活动标签外的所有会话标签（会话标签右键菜单） */
export async function closeOtherSessionTabs(): Promise<void> {
  const keep = store.activeSessionId;
  for (const tab of [...store.sessionTabs]) {
    if (tab.id === keep) continue;
    // 运行中的会话标签跳过（不逐个弹确认），与“关闭所有标签”行为一致
    if (tab.turnActive || tab.goalText) continue;
    await closeSessionTab(tab.id);
  }
}

/**
 * 关闭全部会话标签（“关闭所有标签”用）：运行中的跳过并计数；
 * 允许关闭到 0 个会话标签（主区域显示空状态）。
 * 返回跳过的运行中标签数量。
 */
export async function closeAllSessionTabs(): Promise<number> {
  let skipped = 0;
  for (const tab of [...store.sessionTabs]) {
    if (tab.turnActive || tab.goalText) {
      skipped++;
      continue;
    }
    await closeSessionTab(tab.id);
  }
  return skipped;
}

/**
 * 打开历史会话的统一入口（唯一性约束）：已有标签绑定该线程则直接切换（不新建、
 * 不重载）；否则新建标签并加载。返回 true 表示已进入目标会话。
 */
export async function openSessionTabForThread(
  threadId: string,
): Promise<boolean> {
  if (threadId === store.currentThreadId) return true;
  const existing = findSessionTabByThread(threadId);
  if (existing) {
    await switchSessionTab(existing.id);
    return true;
  }
  const tab = freshSessionTab();
  tab.threadId = threadId; // 先绑定，加载期间也满足唯一性
  store.sessionTabs.push(tab);
  if (!(await switchSessionTab(tab.id))) {
    const i = store.sessionTabs.indexOf(tab);
    if (i >= 0) store.sessionTabs.splice(i, 1);
    return false;
  }
  tab.origin = "history";
  return await loadThreadInto(tab, threadId);
}

/** 仅测试用：清空会话标签状态 */
export function __resetSessionTabsForTest() {
  store.sessionTabs.splice(0, store.sessionTabs.length);
  store.activeSessionId = null;
}

let unlisteners: UnlistenFn[] = [];
let wired = false;

/**
 * 后台临时线程 id 集合（如标题总结用的 ephemeral 线程）。
 * 这些线程的事件只由各自的一次性监听处理，不得进入全局 UI 状态，
 * 否则临时线程的 turn/completed 会把主对话的进行中状态误置为结束。
 */
const backgroundThreadIds = new Set<string>();

function isBackgroundThread(threadId: string | undefined | null): boolean {
  return !!threadId && backgroundThreadIds.has(threadId);
}

export interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
}

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

/** 回合进行中时在任务栏显示不确定进度条，结束后隐藏（非 Tauri 环境静默忽略） */
async function updateTaskbarProgress() {
  try {
    const win = getCurrentWindow();
    await win.setProgressBar({
      status: store.turnActive
        ? ProgressBarStatus.Indeterminate
        : ProgressBarStatus.None,
    });
  } catch {
    // 非 Tauri 环境（如浏览器预览）忽略
  }
}

watch(
  () => store.turnActive,
  () => void updateTaskbarProgress(),
  { immediate: true },
);

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

function isActiveItem(item: ThreadItem): boolean {
  return (
    item.streaming === true ||
    ["in_progress", "inProgress", "pending", "started"].includes(
      String(item.status ?? ""),
    )
  );
}

function bumpActive(threadId: string, delta: number) {
  store.activeWorkByThread[threadId] = Math.max(
    0,
    (store.activeWorkByThread[threadId] ?? 0) + delta,
  );
}

function upsertItem(threadId: string, item: ThreadItem) {
  const arr = (store.itemsByThread[threadId] ??= []);
  let idx = arr.findIndex((x) => x.id === item.id);
  if (idx < 0 && item.type === "userMessage" && item.clientId) {
    // 服务端推送的用户消息与本地乐观插入的通过 clientId 关联，避免重复
    idx = arr.findIndex(
      (x) => x.type === "userMessage" && x.clientId === item.clientId,
    );
  }
  const merged = idx >= 0 ? { ...arr[idx], ...item } : item;
  if (idx >= 0) {
    if (isActiveItem(arr[idx])) bumpActive(threadId, -1);
    arr[idx] = merged;
  } else {
    arr.push(item);
  }
  if (isActiveItem(merged)) bumpActive(threadId, 1);
  store.itemsRev++;
}

function findItem(threadId: string, itemId: string): ThreadItem | undefined {
  return (store.itemsByThread[threadId] ?? []).find((x) => x.id === itemId);
}

function flattenTurns(turns?: Turn[]): ThreadItem[] {
  if (!turns) return [];
  const out: ThreadItem[] = [];
  for (const t of turns) {
    out.push(...(t.items ?? []));
  }
  return out;
}

interface TurnsListPage {
  data: Turn[];
  nextCursor: string | null;
}

/** 用 thread/turns/list(itemsView=full) 拉取历史的完整工具/命令项 */
async function loadFullItems(threadId: string): Promise<ThreadItem[] | null> {
  const turns: Turn[] = [];
  let cursor: string | null = null;
  try {
    for (let i = 0; i < 30; i++) {
      const res = (await invoke("codex_rpc", {
        method: "thread/turns/list",
        params: {
          threadId,
          cursor,
          limit: 50,
          // 协议 SortDirection 枚举为 "asc" | "desc"（"ascending" 会被服务端拒绝）
          sortDirection: "asc",
          itemsView: "full",
        },
      })) as TurnsListPage;
      turns.push(...(res.data ?? []));
      cursor = res.nextCursor ?? null;
      if (!cursor) break;
      if (turns.length > 2000) break; // 防超长会话
    }
  } catch (e) {
    // 服务端不支持时回退到 thread_read 的摘要项；记录原因便于排查协议漂移
    console.warn("[codex-ui] thread/turns/list(full) 失败，回退摘要加载:", e);
    return null;
  }
  return flattenTurns(turns);
}

/**
 * 会话工作区解析（所有入口共用，避免优先级不一致）：
 * 有会话时以会话工作区为准（忽略残留的 newChatWorkspace），无会话（新建会话中）
 * 优先待新建目录，其次启动工作区；均跳过空串。
 * 传 tab 时按指定会话标签解析（后台标签发送回合时沙箱可写根等应跟随该标签）。
 */
export function resolveSessionWorkspace(tab?: SessionTab): string {
  const cwd = tab
    ? tab.threadId
      ? tab.workspace
      : tab.newChatWorkspace
    : store.currentThreadId
      ? store.currentThreadWorkspace
      : store.newChatWorkspace;
  return cwd?.trim() || store.server.startupWorkspace?.trim() || "";
}

/**
 * 当前工作区：由活动编辑器标签决定（文件/diff/预览/终端标签由 EditorPane 写入
 * store.workspace），null 或未设置时回落会话工作区。资源/Git 面板与新建会话初始目录跟随它。
 */
export const workspace = computed(
  () => store.workspace ?? resolveSessionWorkspace(),
);

// 活动会话标签记录自动同步：live 字段（当前活动标签）的任何变化都落回标签记录，
// 保证切走/切回时标签状态不丢失（消息列表本身按线程存于 itemsByThread，无需同步）。
watch(
  () => [
    store.currentThreadId,
    store.currentThreadName,
    store.permissionMode,
    store.taskMode,
    store.model,
    store.effort,
    store.currentThreadOrigin,
    store.currentThreadWorkspace,
    store.resumedThreadId,
    store.turnActive,
    store.currentTurnId,
    store.turnInterrupted,
    store.goalText,
    store.goalStatus,
    store.goalArmed,
    store.threadTokenUsage,
    store.planPrompt,
    store.loading,
    store.newChatWorkspace,
    store.followupQueue.length,
    store.attachments.length,
    store.threads.find((t) => t.id === store.currentThreadId)?.name,
  ],
  () => syncActiveSessionTab(),
);

function isThreadNotFound(e: unknown): boolean {
  return String(e).toLowerCase().includes("thread not found");
}

/** 当前会话已不存在（被删除等）时重置回新对话，避免继续发送一直报错 */
function resetToNewChat() {
  store.currentThreadId = null;
  store.currentThreadName = "";
  store.currentThreadOrigin = null;
  store.currentThreadWorkspace = null;
  store.resumedThreadId = null;
  store.currentTurnId = null;
  store.turnActive = false;
  store.turnInterrupted = false;
  store.goalText = null;
  store.goalStatus = null;
  store.goalArmed = false;
}

export async function loadSettings() {
  try {
    const s = await invoke<AppSettings>("settings_get");
    store.settings = { ...defaultSettings(), ...s };
  } catch {
    store.settings = defaultSettings();
  }
  // 持久化值非法时回退默认；默认权限作为权限模式的启动初始值（运行期切换不写回配置）
  if (
    !PERMISSION_MODES.some((m) => m.id === store.settings.default_permission)
  ) {
    store.settings.default_permission = "ask-for-approval";
  }
  if (!["enabled", "disabled"].includes(store.settings.memory_mode)) {
    store.settings.memory_mode = "disabled";
  }
  store.permissionMode = store.settings.default_permission;
  applyTheme(store.settings.theme);
}

export async function saveSettings(patch: Partial<AppSettings>) {
  store.settings = { ...store.settings, ...patch };
  await invoke("settings_set", { settings: store.settings });
  applyTheme(store.settings.theme);
}

export async function refreshServer() {
  const s = await invoke<ServerStatus>("server_status");
  store.server = { ...store.server, ...s };
}

/** 拉取可用模型列表（幂等），供模型菜单与输入区按钮共用 */
export async function loadModels(force = false) {
  if (store.modelsLoaded && !force) return;
  try {
    const res = await invoke<{ data: ModelInfo[] }>("codex_rpc", {
      method: "model/list",
      params: {},
    });
    store.models = (res.data ?? []).filter((m) => !m.hidden);
    store.modelsLoaded = true;
  } catch {
    // 模型列表不可用时保持空，UI 回退
  }
}

/** 确保指定对话的插件缓存已加载（对话级缓存：已加载直接返回，不回退 skills/list） */
export async function ensureThreadPlugins(threadId: string) {
  if (store.threadPlugins[threadId]?.loaded) return;
  store.threadPlugins[threadId] = { plugins: [], loaded: false };
  try {
    const res = await invoke<{
      marketplaces?: {
        plugins?: {
          id?: string;
          name: string;
          installed?: boolean;
          enabled?: boolean;
          source?: { path?: string };
          interface?: {
            displayName?: string;
            shortDescription?: string;
            longDescription?: string;
            composerIcon?: string;
            composerIconUrl?: string | null;
            brandColor?: string;
          };
        }[];
      }[];
    }>("codex_rpc", {
      method: "plugin/list",
      params: {},
    });
    const list: PluginItem[] = [];
    const seen = new Set<string>();
    for (const mp of res?.marketplaces ?? []) {
      for (const p of mp.plugins ?? []) {
        if (p.installed === false || p.enabled === false) continue;
        const id = p.id ?? p.name;
        if (seen.has(id)) continue;
        seen.add(id);
        list.push({
          id,
          name: p.name,
          displayName: p.interface?.displayName ?? p.name,
          description:
            p.interface?.shortDescription ?? p.interface?.longDescription ?? "",
          path: p.source?.path ?? "",
          iconPath: p.interface?.composerIcon ?? "",
          iconUrl: p.interface?.composerIconUrl ?? "",
          brandColor: p.interface?.brandColor ?? "",
        });
      }
    }
    store.threadPlugins[threadId] = { plugins: list, loaded: true };
  } catch {
    // 插件列表不可用时保持空，不回退 skills/list
    store.threadPlugins[threadId] = { plugins: [], loaded: false };
  }
}

/** 拉取技能列表（全局缓存，幂等），供 $ 菜单与回显悬浮提示使用 */
export async function ensureSkills(force = false) {
  if (store.skillsLoaded && !force) return;
  try {
    const res = await invoke<{
      data?: {
        skills?: (SkillItem & {
          description?: string;
          interface?: { shortDescription?: string };
        })[];
      }[];
    }>("codex_rpc", { method: "skills/list", params: {} });
    const list = (res?.data ?? [])
      .flatMap((d) => d.skills ?? [])
      .filter((s) => (s as { enabled?: boolean }).enabled !== false)
      .map<SkillItem>((s) => ({
        name: s.name,
        key: s.name,
        path: s.path ?? "",
        desc: s.description ?? s.interface?.shortDescription ?? s.desc ?? "",
        shortDesc:
          s.interface?.shortDescription ?? s.description ?? s.desc ?? "",
      }));
    store.skills = list;
    store.skillsLoaded = true;
  } catch {
    // 技能列表不可用时保持空
    store.skills = [];
  }
}

/** 解析模型的显示名：指定模型优先，否则用默认模型 */
export function modelDisplayName(model: string | null): string {
  if (model) {
    const m = store.models.find((x) => x.model === model);
    return m?.displayName || model;
  }
  const def = store.models.find((x) => x.isDefault);
  return def?.displayName || "默认模型";
}

/** 当前生效模型 id：显式选择 → 会话已知模型 → 默认模型 → 列表首个 → 空 */
export function currentModelId(): string {
  if (store.model) return store.model;
  if (store.currentModel) return store.currentModel;
  const def = store.models.find((m) => m.isDefault);
  if (def) return def.model;
  return store.models[0]?.model ?? "";
}

/** 当前生效的推理强度：显式值优先，否则用默认模型的默认强度 */
export function effectiveEffort(): string {
  if (store.effort) return store.effort;
  const m =
    store.models.find((x) => x.model === store.model) ??
    store.models.find((x) => x.isDefault);
  return m?.defaultReasoningEffort ?? "";
}

/** 置顶协议能力（由 codex_pin_capability 探测，覆盖三代 codex 协议） */
export type PinProtocol =
  | "section_move" // 新版：threadSection/move { sectionId }
  | "metadata_section" // 分区时代：thread/metadata/update { sectionId }
  | "metadata_is_pinned" // 旧版：thread/metadata/update { isPinned }
  | "unsupported";

export interface PinCapability {
  protocol: PinProtocol;
  pinnedSectionId: string | null;
  /** section_move 时实际可用的分区移动方法名（新版 codex 为 thread/section/move） */
  sectionMoveMethod?: "threadSection/move" | "thread/section/move";
}

/** 新版协议内置的 “Pinned” 分区，作为置顶的持久化位置（可用 threadSection/list 发现） */
const PINNED_SECTION_NAME = "Pinned";
/** 兜底：该内置分区 id 在所有安装中固定（与 codex 源码常量一致） */
const FALLBACK_PINNED_SECTION_ID = "01984de2-8f74-7c91-a3b2-5c5e937cf318";
let pinCapabilityCache: PinCapability | null = null;

/** 仅测试用：重置置顶能力缓存 */
export function __resetPinnedSectionForTest() {
  pinCapabilityCache = null;
}

/** 线程是否置顶：优先看服务端返回的 section 是否指向内置 Pinned 分区 */
function isPinnedThread(t: ThreadSummary): boolean {
  if (t.isPinned) return true;
  const sid = t.section?.id;
  if (!sid) return false;
  return (
    sid === pinCapabilityCache?.pinnedSectionId ||
    sid === FALLBACK_PINNED_SECTION_ID ||
    t.section?.name === PINNED_SECTION_NAME
  );
}

/** 把服务端 section 状态物化为 isPinned 字段，供现有 UI 与排序直接使用 */
function normalizeThreadPins(list: ThreadSummary[]): ThreadSummary[] {
  return list.map((t) => ({ ...t, isPinned: isPinnedThread(t) }));
}

/** 固定优先，再按最近时间降序 */
export function sortThreads(list: ThreadSummary[]): ThreadSummary[] {
  return normalizeThreadPins(list).sort((a, b) => {
    if (a.isPinned !== b.isPinned) return a.isPinned ? -1 : 1;
    const ar = a.recencyAt ?? a.updatedAt ?? 0;
    const br = b.recencyAt ?? b.updatedAt ?? 0;
    return br - ar;
  });
}

/** 获取当前 codex 的置顶协议能力（首次调用后缓存；探测失败返回 null 以便重试） */
async function getPinCapability(): Promise<PinCapability | null> {
  if (pinCapabilityCache) return pinCapabilityCache;
  try {
    const cap = await invoke<PinCapability>("codex_pin_capability");
    pinCapabilityCache = cap;
    return cap;
  } catch {
    return null;
  }
}

/** 标题总结能力（codex_title_helper_capability 探测结果，覆盖版本差异） */
export interface TitleHelperCapability {
  experimentalApi: boolean;
  ephemeral: boolean;
}

let titleHelperCapabilityCache: TitleHelperCapability | null = null;

/** 仅测试用：重置标题总结能力缓存 */
export function __resetTitleHelperCapabilityForTest() {
  titleHelperCapabilityCache = null;
}

/** 获取当前 codex 的标题总结能力（首次调用后缓存；探测失败返回 null） */
async function getTitleHelperCapability(): Promise<TitleHelperCapability | null> {
  if (titleHelperCapabilityCache) return titleHelperCapabilityCache;
  try {
    const cap = await invoke<TitleHelperCapability>("codex_title_helper_capability");
    titleHelperCapabilityCache = cap;
    return cap;
  } catch {
    return null;
  }
}

/** 全量加载历史会话：逐页拉取直至 cursor 为空（防死循环上限 200 页） */
export async function refreshThreads() {
  if (store.loadingHistory) return;
  store.loadingHistory = true;
  const all: ThreadSummary[] = [];
  let cursor: string | null = null;
  try {
    for (let i = 0; i < 200; i++) {
      const res: {
        data: ThreadSummary[];
        nextCursor: string | null;
      } = await invoke("thread_list", {
        limit: 50,
        cursor,
      });
      const page = res.data ?? [];
      all.push(...page);
      cursor = res.nextCursor ?? null;
      if (!cursor || page.length === 0) break;
    }
    store.threads = sortThreads(all);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    store.loadingHistory = false;
  }
}

/** 搜索历史会话（thread/search）：全量翻页，结果写入 store.threads 并附带摘要 */
export async function searchThreads(term: string) {
  const t = term.trim();
  if (!t) {
    clearSearch();
    return;
  }
  if (store.loadingHistory) return;
  store.loadingHistory = true;
  const all: { thread: ThreadSummary; snippet: string }[] = [];
  let cursor: string | null = null;
  try {
    for (let i = 0; i < 200; i++) {
      const res: {
        data: { thread: ThreadSummary; snippet: string }[];
        nextCursor: string | null;
      } = await invoke("codex_rpc", {
        method: "thread/search",
        params: {
          searchTerm: t,
          limit: 50,
          cursor,
          sourceKinds: ["cli", "vscode", "exec", "appServer", "unknown"],
        },
      });
      const page = res.data ?? [];
      all.push(...page);
      cursor = res.nextCursor ?? null;
      if (!cursor || page.length === 0) break;
    }
    const snippets: Record<string, string> = {};
    for (const r of all) snippets[r.thread.id] = r.snippet ?? "";
    store.searchSnippets = snippets;
    store.threads = sortThreads(all.map((r) => r.thread));
    store.searchActive = true;
  } catch (e) {
    setToast(toastError(e));
  } finally {
    store.loadingHistory = false;
  }
}

/** 退出搜索，恢复常规列表 */
export function clearSearch() {
  store.searchActive = false;
  store.searchSnippets = {};
  void refreshThreads();
}

/**
 * 重命名会话；返回是否成功（手动重命名 / 首条消息作标题 / AI 总结写回共用）。
 * source 决定 nameIsFirstMessage 标记：manual（默认）清除标记，
 * first-message 置位（AI 总结可覆盖）；auto-summary 不修改标记（由调用方维护）。
 */
export async function renameThread(
  threadId: string,
  name: string,
  source: "manual" | "first-message" | "auto-summary" = "manual",
): Promise<boolean> {
  const n = name.trim();
  if (!n) return false;
  try {
    await invoke("thread_set_name", { threadId, name: n });
    const t = store.threads.find((x) => x.id === threadId);
    if (t) t.name = n;
    const tab = findSessionTabByThread(threadId);
    if (tab) {
      tab.name = n;
      if (source !== "auto-summary") {
        tab.nameIsFirstMessage = source === "first-message";
      }
      tab.title = sessionTabTitle(tab);
    }
    if (store.currentThreadId === threadId) {
      store.currentThreadName = n;
    }
    return true;
  } catch (e) {
    setToast(toastError(e));
    return false;
  }
}

/** 清洗模型生成的标题：去引号/Markdown 标记、折叠空白、截断 50 字 */
export function sanitizeTitle(raw: string): string {
  const t = raw
    .replace(/[`*_#>]/g, "")
    .replace(/^["'“”‘’\s]+|["'“”‘’\s]+$/g, "")
    .replace(/\s+/g, " ")
    .trim();
  if (!t) return "";
  return t.length > 50 ? t.slice(0, 50) : t;
}

/**
 * 仿 VS Code：临时线程总结首条消息，为会话生成短标题（不指定模型，用默认模型）。
 * 与主回合并行执行、失败静默（保留默认标题）；支持 experimentalApi 才执行，
 * 不支持 ephemeral 时退化为普通线程总结后删除。
 */
export async function autoTitleThread(threadId: string, firstMessagePlain: string) {
  const text = firstMessagePlain.replace(/\s+/g, " ").trim();
  if (!text || text.length <= 15) return; // 短文保持默认标题，不消耗模型
  const tab = findSessionTabByThread(threadId);
  // 仅当尚无名称、或名称来自首条消息（可被总结覆盖）时继续；手动命名不覆盖
  if (tab?.name && !tab.nameIsFirstMessage) return;
  const t = store.threads.find((x) => x.id === threadId);
  if ((!tab?.name || !tab.nameIsFirstMessage) && (t?.name || store.currentThreadName)) return;
  const cap = await getTitleHelperCapability();
  if (!cap?.experimentalApi) return; // 不支持 experimentalApi：不总结

  let helperThreadId: string | null = null;
  let helperTurnId: string | null = null;
  let settled = false;
  const oneOff: UnlistenFn[] = [];
  let titleText = "";
  const titleByItem = new Map<string, string>();
  let timer: ReturnType<typeof setTimeout> | null = null;

  const releaseOneOff = () => {
    for (const un of oneOff) {
      try {
        un();
      } catch {
        // 忽略注销失败
      }
    }
    oneOff.length = 0;
  };

  const cleanup = async () => {
    if (!helperThreadId) return;
    backgroundThreadIds.delete(helperThreadId);
    try {
      if (cap.ephemeral) {
        await invoke("codex_rpc", {
          method: "thread/unsubscribe",
          params: { threadId: helperThreadId },
        });
      } else {
        await invoke("thread_delete", { threadId: helperThreadId });
      }
    } catch {
      // 清理失败不影响主会话
    }
  };

  const finish = async (status?: string) => {
    if (settled) return;
    settled = true;
    if (timer) clearTimeout(timer);
    releaseOneOff();
    if (status === "completed") {
      const title = sanitizeTitle(titleText);
      if (title) {
        // 目标线程若已在总结期间被手动命名，不再覆盖；首条消息名可覆盖
        const cur = findSessionTabByThread(threadId);
        if (!cur?.name || cur.nameIsFirstMessage) {
          if (await renameThread(threadId, title, "auto-summary")) {
            if (cur) cur.nameIsFirstMessage = false;
            setToast("当前会话的标题已简化");
            // 历史列表同步最终标题（搜索态下不覆盖搜索结果）
            if (!store.searchActive) void refreshThreads();
          }
        }
      }
    }
    await cleanup();
  };

  try {
    const startParams: Record<string, unknown> = {
      cwd: resolveSessionWorkspace(),
      approvalPolicy: "never",
      sandbox: "read-only",
    };
    if (cap.ephemeral) {
      startParams.ephemeral = true;
    }
    const started = await invoke<{ thread?: { id?: string } }>("thread_start", {
      params: startParams,
    });
    helperThreadId = started?.thread?.id ?? null;
    if (!helperThreadId) return;
    backgroundThreadIds.add(helperThreadId);

    oneOff.push(
      await listen("item/agentMessage/delta", (e) => {
        const p = e.payload as {
          threadId?: string;
          itemId?: string;
          delta?: string;
        };
        if (p.threadId !== helperThreadId || !p.itemId) return;
        titleByItem.set(
          p.itemId,
          (titleByItem.get(p.itemId) ?? "") + (p.delta ?? ""),
        );
        // 标题取最后一个 agentMessage 的累积文本
        titleText = [...titleByItem.values()].pop() ?? titleText;
      }),
      await listen("turn/started", (e) => {
        const p = e.payload as {
          threadId?: string;
          turn?: { id?: string };
        };
        if (p.threadId !== helperThreadId) return;
        helperTurnId = p.turn?.id ?? null;
      }),
      await listen("turn/completed", (e) => {
        const p = e.payload as {
          threadId?: string;
          turn?: { id?: string; status?: string };
        };
        if (p.threadId !== helperThreadId) return;
        void finish(p.turn?.status);
      }),
    );

    // 超时兜底：尽力中断临时回合并清理，标题保持默认
    timer = setTimeout(() => {
      if (settled) return;
      void (async () => {
        if (helperThreadId && helperTurnId) {
          try {
            await invoke("turn_interrupt", {
              threadId: helperThreadId,
              turnId: helperTurnId,
            });
          } catch {
            // 中断失败不阻塞清理
          }
        }
        await finish();
      })();
    }, 30_000);

    const input = buildTurnInput(
      `给下面用户消息生成一个不超过 30 字的中文会话标题，只输出标题本身，不要任何解释、引号或 Markdown。\n\n用户消息：\n${text}`,
      [],
    );
    await invoke("turn_start", {
      params: {
        threadId: helperThreadId,
        input,
        approvalPolicy: "never",
        sandboxPolicy: { type: "readOnly", networkAccess: false },
      },
    });
  } catch {
    settled = true;
    releaseOneOff();
    await cleanup();
  }
}

/** 固定/取消固定会话（置顶） */
export async function togglePin(threadId: string, pinned: boolean) {
  const t = store.threads.find((x) => x.id === threadId);
  const prev = t?.isPinned;
  try {
    const cap = await getPinCapability();
    if (!cap || cap.protocol === "unsupported") {
      setToast("当前 Codex 版本不支持置顶");
      return;
    }
    if (t) t.isPinned = pinned;
    if (cap.protocol === "metadata_is_pinned") {
      // 旧版协议：isPinned 布尔元数据
      await invoke("codex_rpc", {
        method: "thread/metadata/update",
        params: { threadId, isPinned: pinned },
      });
    } else {
      const sectionId = pinned ? cap.pinnedSectionId : null;
      if (pinned && !sectionId) {
        if (t) t.isPinned = prev;
        setToast("当前 Codex 版本不支持置顶");
        return;
      }
      if (cap.protocol === "section_move") {
        // 新版协议：threadSection/move（0.147+ 改名为 thread/section/move，按探测结果调用）
        const method = cap.sectionMoveMethod ?? "threadSection/move";
        await invoke("codex_rpc", {
          method,
          params: { threadId, sectionId },
        });
      } else {
        // 分区时代协议：metadata/update 携带 sectionId
        await invoke("codex_rpc", {
          method: "thread/metadata/update",
          params: { threadId, sectionId },
        });
      }
    }
    await refreshThreads();
  } catch (e) {
    if (t) t.isPinned = prev;
    setToast(toastError(e));
  }
}

async function newChat(prompt: string, attachments: UserInput[]) {
  store.busy = true;
  const tabId = store.activeSessionId;
  try {
    // newChat 仅在无当前会话时被调用，resolveSessionWorkspace 走 newChatWorkspace → workspace 分支
    const cwd = resolveSessionWorkspace();
    const params: Record<string, unknown> = {
      cwd,
      approvalPolicy: toApprovalPolicy(store.permissionMode),
      sandbox: toSandbox(store.permissionMode),
    };
    const reviewer = toApprovalsReviewer(store.permissionMode);
    if (reviewer) params.approvalsReviewer = reviewer;
    // 显式携带（null 表示用默认），避免旧值在会话里粘滞；effort 由随后的 turn/start 携带
    params.model = store.model ?? null;
    const res = await invoke<{ thread: { id: string; name?: string | null }; model?: string }>(
      "thread_start",
      { params },
    );
    const threadId = res.thread.id;
    // 防御：该新线程已被其它标签绑定（异常路径），聚焦已有标签并释放当前标签
    const existing = findSessionTabByThread(threadId);
    if (existing) {
      const cur = activeSessionTab();
      const i = cur ? store.sessionTabs.indexOf(cur) : -1;
      if (i >= 0) store.sessionTabs.splice(i, 1);
      store.activeSessionId = existing.id;
      restoreSession(existing);
      store.currentModel = res.model ?? currentModelId();
      await refreshThreads();
      return;
    }
    // 创建期间用户已切换到其它标签：结果直接写入原标签记录，避免污染当前会话
    if (store.activeSessionId !== tabId) {
      const tab = store.sessionTabs.find((t) => t.id === tabId);
      if (tab) {
        tab.threadId = threadId;
        tab.name = res.thread.name ?? "";
        tab.origin = "new";
        tab.workspace = cwd;
        tab.resumedThreadId = threadId;
        tab.newChatWorkspace = null;
        tab.loading = false;
        tab.title = sessionTabTitle(tab);
      }
      store.itemsByThread[threadId] = [];
      store.activeWorkByThread[threadId] = 0;
      store.currentModel = res.model ?? currentModelId();
      await refreshThreads();
      return;
    }
    store.currentThreadId = threadId;
    store.currentThreadName = res.thread.name ?? "";
    store.currentThreadOrigin = "new";
    store.currentThreadWorkspace = cwd;
    store.resumedThreadId = threadId;
    store.newChatWorkspace = null; // 本次新建已消费，恢复默认
    store.currentModel = res.model ?? currentModelId();
    void ensureThreadPlugins(threadId); // 进入新对话即预初始化插件缓存
    store.itemsByThread[threadId] = [];
    store.activeWorkByThread[threadId] = 0;
    // 待挂载目标（勾选后首条消息即目标）：创建会话后挂载到新线程；失败不阻塞新建，
    // 清空本地目标状态，用户可重新勾选
    const pendingGoal = store.goalText;
    if (pendingGoal) {
      const ok = await setGoal(pendingGoal);
      if (!ok) {
        store.goalText = null;
        store.goalStatus = null;
        store.goalArmed = false;
      }
    }
    // 记忆模式：显式应用持久化设置（含关闭），保证新会话与设置一致；失败静默跳过
    try {
      await invoke("codex_rpc", {
        method: "thread/memoryMode/set",
        params: { threadId, mode: store.settings.memory_mode },
      });
    } catch {
      // 服务端不支持记忆特性时静默跳过，不打扰新建流程
    }
    await refreshThreads();
    if (prompt.trim() || attachments.length) {
      // 第 2 步：首条消息内容作为会话标题（AI 总结完成后由 autoTitleThread 覆盖）
      const firstText = stripMentionContext(prompt).replace(/\s+/g, " ").trim();
      if (firstText) {
        const tab = findSessionTabByThread(threadId);
        if (
          await renameThread(threadId, sanitizeTitle(firstText), "first-message")
        ) {
          if (tab) tab.nameIsFirstMessage = true;
        }
      }
      // 仿 VS Code：后台临时线程总结首条消息生成短标题（不阻塞主回合）
      void autoTitleThread(threadId, stripMentionContext(prompt));
      await continueTurn(prompt, attachments);
    }
  } finally {
    store.busy = false;
  }
}

/**
 * 组装 turn/start 参数：权限/沙箱/模型/推理强度/协作模式按当前全局设置，
 * 沙箱可写根跟随传入的 cwd（活动标签用 resolveSessionWorkspace，后台标签用 resolveSessionWorkspace）。
 */
function buildTurnParams(
  threadId: string,
  input: UserInput[],
  clientId: string,
  cwd: string,
): Record<string, unknown> {
  const params: Record<string, unknown> = {
    threadId,
    input,
    clientUserMessageId: clientId,
  };
  // 权限模式随每一轮发送（协议：本回合及后续回合生效），空闲期切换后立即生效
  params.approvalPolicy = toApprovalPolicy(store.permissionMode);
  params.sandboxPolicy = toSandboxPolicy(store.permissionMode, cwd);
  const reviewer = toApprovalsReviewer(store.permissionMode);
  if (reviewer) params.approvalsReviewer = reviewer;
  // 显式携带（null 表示用默认），避免旧值在会话里粘滞
  params.model = store.model ?? null;
  params.effort = store.effort ?? null;
  // 协作模式会粘滞在会话上：计划模式需要显式切回 default 才能退出；
  // 因此每轮都显式携带当前任务模式对应的 collaborationMode。
  // 模型未知时绝不发送空字符串（上游会报 invalid_request_error），此时省略该字段。
  const collabModel = currentModelId();
  if (collabModel) {
    params.collaborationMode = {
      mode: store.taskMode === "plan" ? "plan" : "default",
      settings: {
        model: collabModel,
        reasoning_effort: store.effort ?? null,
        developer_instructions: null,
      },
    };
  }
  return params;
}

/** 按标签发送回合（后台标签的队列消息等用）：状态写入目标标签记录，不触碰活动标签 */
async function continueTurnForTab(
  tab: SessionTab,
  prompt: string,
  attachments: UserInput[],
) {
  const threadId = tab.threadId;
  if (!threadId) return;
  // 后台历史会话同样按需恢复；新会话（thread/start 创建）已订阅无需恢复
  if (tab.resumedThreadId !== threadId) {
    try {
      await invoke("thread_resume", { params: { threadId } });
      tab.resumedThreadId = threadId;
    } catch (e) {
      if (isThreadNotFound(e)) {
        const i = store.sessionTabs.indexOf(tab);
        if (i >= 0) store.sessionTabs.splice(i, 1);
        setToast("会话已不存在，已关闭该标签");
      } else {
        setToast(toastError(e));
      }
      return;
    }
  }
  const clientId = `user-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const input = buildTurnInput(prompt, attachments);
  const params = buildTurnParams(threadId, input, clientId, resolveSessionWorkspace(tab));
  upsertItem(threadId, {
    id: clientId,
    clientId,
    type: "userMessage",
    content: input,
    startedAtMs: Date.now(),
  });
  // 待挂载目标：先挂载再启动回合，失败清空该标签目标状态
  if (tab.goalText && !tab.goalStatus) {
    try {
      await invoke("goal_set", { threadId, objective: tab.goalText });
      tab.goalStatus = "active";
    } catch (e) {
      tab.goalText = null;
      tab.goalStatus = null;
      tab.goalArmed = false;
      setToast(toastError(e));
    }
  }
  try {
    const res = await invoke<{ turn?: { id?: string } }>("turn_start", {
      params,
    });
    tab.turnActive = true;
    if (res?.turn?.id) tab.currentTurnId = res.turn.id;
  } catch (e) {
    if (isThreadNotFound(e)) {
      const i = store.sessionTabs.indexOf(tab);
      if (i >= 0) store.sessionTabs.splice(i, 1);
      setToast("会话已不存在，已关闭该标签");
    } else {
      setToast(toastError(e));
    }
    tab.turnActive = false;
  }
}

async function continueTurn(prompt: string, attachments: UserInput[]) {
  const threadId = store.currentThreadId;
  if (!threadId) return;
  // 历史会话在打开时只读、不恢复，避免带活跃目标的会话被自动持续执行；
  // 用户真正发消息时才恢复（thread/start 新建的会话已订阅，无需恢复）。
  if (store.resumedThreadId !== threadId) {
    try {
      await invoke("thread_resume", { params: { threadId } });
      store.resumedThreadId = threadId;
    } catch (e) {
      if (isThreadNotFound(e)) {
        resetToNewChat();
        setToast("会话已不存在，已切换为新会话");
      } else {
        setToast(toastError(e));
      }
      return;
    }
  }
  const clientId = `user-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  // 与 VS Code Codex 扩展一致：文件引用序列化成文本段落，作为单条 text 输入
  const input = buildTurnInput(prompt, attachments);
  const params = buildTurnParams(threadId, input, clientId, resolveSessionWorkspace());
  upsertItem(threadId, {
    id: clientId,
    clientId,
    type: "userMessage",
    content: input,
    startedAtMs: Date.now(),
  });
  // 待挂载目标（勾选后首条消息即目标）：先挂载再启动回合，服务端按目标线程自动续跑；
  // 挂载失败清空本地目标状态（toast 已由 setGoal 提示），不阻塞回合
  if (store.goalText && !store.goalStatus) {
    const ok = await setGoal(store.goalText);
    if (!ok) {
      store.goalText = null;
      store.goalStatus = null;
      store.goalArmed = false;
    }
  }
  try {
    const res = await invoke<{ turn?: { id?: string } }>("turn_start", { params });
    store.turnActive = true;
    // 立即记录回合 id，供 turn/interrupt 使用（turn/started 事件可能稍后才到）
    if (res?.turn?.id) store.currentTurnId = res.turn.id;
  } catch (e) {
    if (isThreadNotFound(e)) {
      resetToNewChat();
      setToast("会话已不存在，已切换为新会话");
    } else {
      setToast(toastError(e));
    }
    store.turnActive = false;
  }
}

/** 向进行中的回合追加输入（“调整方向”），协议 turn/steer */
async function steerTurn(prompt: string, attachments: UserInput[]) {
  const threadId = store.currentThreadId;
  if (!threadId || !store.currentTurnId) {
    setToast("当前没有进行中的回合");
    return;
  }
  const clientId = `user-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const input = buildTurnInput(prompt, attachments);
  upsertItem(threadId, {
    id: clientId,
    clientId,
    type: "userMessage",
    content: input,
    startedAtMs: Date.now(),
  });
  try {
    await invoke("turn_steer", {
      params: {
        threadId,
        clientUserMessageId: clientId,
        input,
        expectedTurnId: store.currentTurnId,
      },
    });
  } catch (e) {
    const msg = String(e);
    if (msg.includes("no active turn")) {
      // 服务端在 turn/start 响应与 turn/started 事件之间可能尚未把回合
      // 标记为可转向；短暂重试几次，避免“no active turn to steer”导致输入丢失。
      for (let attempt = 0; attempt < 5; attempt++) {
        await new Promise((r) => setTimeout(r, 400));
        try {
          await invoke("turn_steer", {
            params: {
              threadId,
              clientUserMessageId: clientId,
              input,
              expectedTurnId: store.currentTurnId,
            },
          });
          return;
        } catch (e2) {
          const m2 = String(e2);
          if (!m2.includes("no active turn")) {
            setToast(toastError(e2));
            return;
          }
        }
      }
    }
    setToast(toastError(e));
  }
}

export async function sendPrompt(text: string, flip = false) {
  const attachments = store.attachments.splice(0);
  if (!text.trim() && attachments.length === 0) return;
  // 手动发送标记：ChatView 据此在发送后强制恢复吸底回到底部
  // （队列消息在回合结束后自动发送时走 continueTurn/newChat，不递增）
  store.userSendRev++;
  // 回合进行中：按“跟进处理方式”转向或入队；Ctrl+Enter 对单条消息取相反方式
  if (store.turnActive && store.currentThreadId) {
    const base = store.settings.followup_mode;
    const mode = flip ? (base === "adjust" ? "queue" : "adjust") : base;
    if (mode === "adjust") {
      await steerTurn(text, attachments);
    } else {
      store.followupQueue.push({ text, attachments });
      setToast("已加入队列，回合结束后自动发送");
    }
    return;
  }
  try {
    if (!store.currentThreadId) {
      await newChat(text, attachments);
    } else {
      await continueTurn(text, attachments);
    }
  } catch (e) {
    setToast(toastError(e));
    store.busy = false;
  }
}

/** “待在计划”：关闭“计划已就绪”弹窗，保持计划模式，不发消息（Esc 同此行为） */
export function dismissPlanPrompt() {
  store.planPrompt = null;
}

/** “退出计划模式”：切回执行模式并关闭弹窗，不发消息 */
export function exitPlanMode() {
  store.planPrompt = null;
  store.taskMode = "execute";
}

/** “执行计划”：仿 VS Code —— 发送 `PLEASE IMPLEMENT THIS PLAN:` 消息并切到执行模式 */
export async function executePlan() {
  const prompt = store.planPrompt;
  if (!prompt) return;
  store.planPrompt = null;
  // 先切模式，使本轮 turn/start 显式携带 collaborationMode default（计划模式粘滞，需显式退出）
  store.taskMode = "execute";
  const text = `PLEASE IMPLEMENT THIS PLAN:\n${prompt.planText}`;
  // 目标勾选：执行计划即首条执行消息，目标=该合成消息（含计划全文）
  if (store.goalArmed) {
    store.goalText = text;
    store.goalArmed = false;
    store.goalStatus = null;
  }
  try {
    if (store.currentThreadId) {
      await continueTurn(text, []);
    } else {
      await newChat(text, []);
    }
  } catch (e) {
    setToast(toastError(e));
    store.busy = false;
  }
}

/**
 * 新建空会话（所有 UI 入口的统一函数）：可预置本次会话的工作目录 cwd。
 * 返回 true 表示已进入新会话；进行中会话确认被取消时返回 false（不切换）。
 */
/**
 * 新建空会话标签（所有“新会话”入口的统一函数）：会话多开，不打断/不停止
 * 当前或后台标签的回合；可预置本次会话的工作目录 cwd。恒返回 true。
 */
export async function newEmptyChat(cwd?: string | null): Promise<boolean> {
  const tab = freshSessionTab();
  if (cwd) tab.newChatWorkspace = cwd;
  tab.title = sessionTabTitle(tab);
  store.sessionTabs.push(tab);
  store.activeSessionId = tab.id;
  restoreSession(tab);
  void ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY); // 进入新对话编辑态即预初始化插件缓存
  return true;
}

/**
 * 打开历史会话：返回 true 表示成功切换到目标会话；
 * 点击当前会话（无操作）或进行中会话确认被取消时返回 false。
 */
/**
 * 把历史会话加载进指定会话标签（结果按“是否仍为活动标签”写入 live 字段或标签记录，
 * 避免加载期间用户切换标签导致状态串味）。
 */
async function loadThreadInto(tab: SessionTab, threadId: string): Promise<boolean> {
  const isActive = () => store.activeSessionId === tab.id;
  tab.loading = true;
  if (isActive()) store.loading = true;
  void ensureThreadPlugins(threadId); // 进入历史对话即预初始化插件缓存
  try {
    const res = await invoke<{
      thread: {
        id: string;
        name?: string | null;
        preview?: string;
        cwd?: string | null;
        turns?: Turn[];
      };
    }>("thread_read", { threadId, includeTurns: true });
    // 优先用全量 items（含命令/工具详情），失败则回退摘要
    const fullItems = await loadFullItems(threadId);
    store.itemsByThread[threadId] = fullItems ?? flattenTurns(res.thread.turns);
    store.activeWorkByThread[threadId] = (
      store.itemsByThread[threadId] ?? []
    ).filter((x) => isActiveItem(x)).length;
    const name = res.thread.name ?? "";
    const cwd = res.thread.cwd ?? null;
    if (isActive()) {
      store.currentThreadId = threadId;
      store.currentThreadName = name;
      store.currentThreadWorkspace = cwd;
      store.resumedThreadId = null; // 只读打开，不恢复；发消息时才恢复
      store.turnActive = false;
      store.turnInterrupted = false;
      store.currentTurnId = null;
      store.threadTokenUsage = null;
      store.goalArmed = false; // 勾选态不跨会话；服务端目标经 goal_get 回填
    } else {
      tab.threadId = threadId;
      tab.name = name;
      tab.workspace = cwd;
      tab.resumedThreadId = null;
      tab.turnActive = false;
      tab.turnInterrupted = false;
      tab.currentTurnId = null;
      tab.threadTokenUsage = null;
      tab.goalArmed = false;
      tab.title = sessionTabTitle(tab);
    }
    let goalText: string | null = null;
    let goalStatus: GoalStatus | null = null;
    try {
      const g = await invoke<{
        objective?: string;
        status?: string;
        goal?: { objective?: string; status?: string };
      }>("goal_get", { threadId });
      const goal = g?.goal ?? g;
      goalText = goal?.objective ?? null;
      goalStatus = isGoalStatus(goal?.status) ? goal.status : null;
    } catch {
      // 服务端不支持/失败：按无目标处理
    }
    // 终态目标（已完成/受限等）：打开即 toast 提示并复位（相当于没有目标），服务端同步清除
    if (goalStatus && isGoalTerminalStatus(goalStatus)) {
      setToast(goalStatusToast(goalStatus));
      goalText = null;
      goalStatus = null;
      void clearGoal(threadId);
    }
    if (isActive()) {
      store.goalText = goalText;
      store.goalStatus = goalStatus;
    } else {
      tab.goalText = goalText;
      tab.goalStatus = goalStatus;
    }
    return true;
  } catch (e) {
    if (isThreadNotFound(e)) {
      if (isActive()) {
        resetToNewChat();
        setToast("会话已不存在，已切换为新会话");
      } else {
        const i = store.sessionTabs.indexOf(tab);
        if (i >= 0) store.sessionTabs.splice(i, 1);
        setToast("会话已不存在，已关闭该标签");
      }
    } else {
      setToast(toastError(e));
    }
    return false;
  } finally {
    tab.loading = false;
    if (isActive()) store.loading = false;
  }
}

/**
 * 打开历史会话：统一走 openSessionTabForThread（唯一性约束：已打开则聚焦，
 * 未打开则新建标签加载）。返回 true 表示成功切换到目标会话。
 */
export async function openThread(threadId: string): Promise<boolean> {
  return openSessionTabForThread(threadId);
}

/** 切换会话成功后的统一收尾：关设置页 → 聚焦输入框（右侧面板保持当前 Tab） */
async function finishSessionSwitch() {
  store.showSettings = false;
  await nextTick();
  focusComposer();
}

/**
 * 新建会话统一入口（头部按钮 / 历史目录右键「新建会话」）：
 * 切换成功（未被取消）才聚焦输入框；右侧面板保持当前 Tab。
 */
export async function openNewSession(cwd?: string | null): Promise<void> {
  if (!(await newEmptyChat(cwd))) return;
  await finishSessionSwitch();
}

/** 目录选择对话框打开中：禁止重复触发（头部「+」与空状态按钮共用，供按钮禁用态绑定） */
export const pickingNewSessionDir = ref(false);

/**
 * 选择工作目录并新建会话（头部「+」与零会话空状态按钮共用）：
 * 先弹目录选择（初始定位当前维护的工作目录），取消则流程直接结束。
 */
export async function pickAndOpenNewSession(): Promise<void> {
  if (pickingNewSessionDir.value) return;
  pickingNewSessionDir.value = true;
  try {
    const dir = await invoke<string | null>("pick_directory", {
      initialDir: workspace.value,
    });
    if (!dir) return;
    await openNewSession(dir);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    pickingNewSessionDir.value = false;
  }
}

/**
 * 打开历史会话统一入口（会话行单击 / 右键「打开」）：
 * 切换成功（未被取消）才聚焦输入框；右侧面板保持当前 Tab。
 */
export async function openHistorySession(threadId: string): Promise<void> {
  if (!(await openThread(threadId))) return;
  await finishSessionSwitch();
}

/** 标准停止回合：与停止按钮一致，线程有活跃目标时先清目标再 turn/interrupt；
 * 可显式传入线程/回合 id（切换会话时用），缺省时操作当前会话。 */
export async function interrupt(
  threadId?: string | null,
  turnId?: string | null,
) {
  const tid = threadId ?? store.currentThreadId;
  if (!tid) return;
  const tab = threadId ? findSessionTabByThread(tid) : activeSessionTab();
  // 线程有活跃目标：先清除目标切断服务端 auto-continuation（目标循环回合极快，
  // 回合中断可能追不上；清除目标后当前回合自然结束、不再自动续跑）
  const hasGoal = tab ? Boolean(tab.goalText) : Boolean(store.goalText);
  if (hasGoal) {
    await clearGoal(tid);
    // 后台标签的目标状态直接清本地（clearGoal 仅清活动会话本地状态）
    if (tab && tab.threadId === tid) {
      tab.goalText = null;
      tab.goalStatus = null;
      tab.goalArmed = false;
    }
  }
  let target = turnId ?? store.currentTurnId;
  if (!target) return;
  // 服务端可能在 turn/started 事件之后才把回合标记为 active；
  // 若用户点得过早会收到 “no active turn”，短暂重试几次。
  for (let attempt = 0; attempt < 5; attempt++) {
    try {
      await invoke("turn_interrupt", {
        threadId: tid,
        turnId: target,
      });
      return;
    } catch (e) {
      const msg = String(e);
      // 回合 id 不一致时，错误里的 “but found X” 是服务端当前活跃回合 id，用它重试
      const found = /but found ([0-9a-fA-F-]+)/i.exec(msg);
      if (found && found[1] !== target) {
        target = found[1];
        // 仅在操作当前会话时同步 store，避免切换会话后把旧回合 id 写进新会话
        if (threadId === undefined) store.currentTurnId = target;
        continue;
      }
      if (msg.includes("no active turn")) {
        await new Promise((r) => setTimeout(r, 400));
        continue;
      }
      setToast(msg);
      return;
    }
  }
}

/** 为当前线程挂载目标（thread/goal/set）；目标设置成功后由服务端自动续跑回合 */
export async function setGoal(objective: string): Promise<boolean> {
  const text = objective.trim();
  if (!text) {
    setToast("目标不能为空");
    return false;
  }
  if (text.length > 4000) {
    setToast("目标最长 4000 字符");
    return false;
  }
  const tid = store.currentThreadId;
  if (!tid) return false; // 仅在线程存在时挂载（防御：调用方应保证有会话）
  try {
    await invoke("goal_set", { threadId: tid, objective: text });
    store.goalText = text;
    store.goalStatus = "active";
    setToast("已设置目标");
    return true;
  } catch (e) {
    setToast(toastError(e));
    return false;
  }
}

export async function clearGoal(threadId?: string | null) {
  const tid = threadId ?? store.currentThreadId;
  if (!tid) {
    store.goalText = null;
    store.goalStatus = null;
    store.goalArmed = false;
    return;
  }
  try {
    await invoke("goal_clear", { threadId: tid });
    // 仅当清除的是当前会话时才清空本地目标展示；
    // 切换会话时对旧线程的清除不应污染新会话的目标状态
    if (tid === store.currentThreadId) {
      store.goalText = null;
      store.goalStatus = null;
      store.goalArmed = false;
    }
  } catch (e) {
    setToast(toastError(e));
  }
}

export async function deleteThread(threadId: string) {
  try {
    await invoke("thread_delete", { threadId });
    store.threads = store.threads.filter((t) => t.id !== threadId);
    // 释放该会话的本地缓存，避免历史列表长期累积内存
    delete store.itemsByThread[threadId];
    delete store.activeWorkByThread[threadId];
    // 关闭绑定该线程的会话标签（线程已删除，无需确认/中断）
    for (const tab of store.sessionTabs.filter((t) => t.threadId === threadId)) {
      const idx = store.sessionTabs.indexOf(tab);
      store.sessionTabs.splice(idx, 1);
      if (store.activeSessionId === tab.id) {
        const next =
          store.sessionTabs[Math.min(idx, store.sessionTabs.length - 1)] ?? null;
        if (next) {
          store.activeSessionId = next.id;
          restoreSession(next);
        } else {
          // 允许 0 个会话标签：复位 live 字段到无会话默认态
          resetLiveSessionState();
        }
      }
    }
  } catch (e) {
    setToast(toastError(e));
  }
}

export async function respondInteraction(interaction: PendingInteraction, result: unknown) {
  try {
    await invoke("interaction_respond", {
      requestId: interaction.requestId,
      result,
    });
  } catch (e) {
    setToast(toastError(e));
  } finally {
    store.interactions = store.interactions.filter((i) => i.requestId !== interaction.requestId);
    for (const tab of store.sessionTabs) {
      if (tab.interactions?.some((i) => i.requestId === interaction.requestId)) {
        tab.interactions = tab.interactions.filter(
          (i) => i.requestId !== interaction.requestId,
        );
      }
    }
  }
}

export async function wireEvents() {
  if (wired) return;
  wired = true;

  unlisteners.push(
    await listen("server/status", (e) => {
      store.server = { ...store.server, ...(e.payload as ServerStatus) };
    }),
  );

  unlisteners.push(
    await listen("interaction:request", async (e) => {
      const p = e.payload as {
        requestId: number | string;
        method: string;
        params: Record<string, unknown>;
      };
      // 按线程路由到对应会话标签（协议确认审批/提问/elicitation 均带 threadId）；
      // 无 threadId 或线程未打开时回退全局列表（由活动标签展示）
      const threadId =
        typeof p.params?.threadId === "string" ? p.params.threadId : undefined;
      const tab = findSessionTabByThread(threadId);
      if (tab) {
        tab.interactions.push({ ...p, at: Date.now() });
      } else {
        store.interactions.push({ ...p, at: Date.now() });
      }
      if (store.settings.sound_enabled) playNotificationSound();
    }),
  );

  unlisteners.push(
    await listen("serverRequest/resolved", (e) => {
      const p = e.payload as { requestId: number | string };
      store.interactions = store.interactions.filter((i) => i.requestId !== p.requestId);
      for (const tab of store.sessionTabs) {
        if (tab.interactions.some((i) => i.requestId === p.requestId)) {
          tab.interactions = tab.interactions.filter(
            (i) => i.requestId !== p.requestId,
          );
        }
      }
    }),
  );

  unlisteners.push(
    await listen("turn/started", (e) => {
      const p = e.payload as { threadId?: string; turn?: { id?: string } };
      if (isBackgroundThread(p.threadId)) return; // 后台临时线程事件不进入全局状态
      const tab = p.threadId
        ? findSessionTabByThread(p.threadId)
        : activeSessionTab();
      if (!tab) return;
      const isActive = store.activeSessionId === tab.id;
      if (isActive) {
        store.turnActive = true;
        store.turnInterrupted = false;
        store.planPrompt = null; // 新回合开始：关闭“计划已就绪”确认弹窗
        // currentTurnId 保留 turn/start 响应的服务端回合 id（turn_interrupt 需要）；
        // 事件 id 仅在响应缺失时兜底。
        if (!store.currentTurnId && p.turn?.id) store.currentTurnId = p.turn.id;
      } else {
        tab.turnActive = true;
        tab.turnInterrupted = false;
        tab.planPrompt = null;
        if (!tab.currentTurnId && p.turn?.id) tab.currentTurnId = p.turn.id;
      }
    }),
    await listen("turn/completed", async (e) => {
      const p = e.payload as {
        threadId?: string;
        turn?: { id?: string; status?: string };
      };
      if (isBackgroundThread(p.threadId)) return; // 后台临时线程完成不影响主对话
      const tid = p.threadId ?? store.currentThreadId;
      const tab = tid ? findSessionTabByThread(tid) : activeSessionTab();
      const interrupted = p.turn?.status === "interrupted";
      const isActive = tab ? store.activeSessionId === tab.id : false;
      if (tab) {
        if (isActive) {
          store.turnActive = false;
          store.turnInterrupted = interrupted;
          store.currentTurnId = null;
        } else {
          tab.turnActive = false;
          tab.turnInterrupted = interrupted;
          tab.currentTurnId = null;
        }
      }
      // 回合结束：把仍处于进行中/流式状态的 item 收敛为终态并补算耗时，
      // 避免手动停止后最后一张工具卡的实时计时持续跳动
      if (tid) {
        const threadItems = store.itemsByThread[tid] ?? [];
        const now = Date.now();
        let touched = false;
        for (const it of threadItems) {
          const s = String(it.status ?? "");
          if (
            it.streaming === true ||
            s === "in_progress" ||
            s === "inProgress" ||
            s === "pending" ||
            s === "started"
          ) {
            if (isActiveItem(it)) bumpActive(tid, -1);
            it.streaming = false;
            it.status = interrupted ? "interrupted" : "canceled";
            if (typeof it.durationMs !== "number") {
              const started =
                typeof it.startedAtMs === "number" ? (it.startedAtMs as number) : now;
              it.durationMs = now - started;
            }
            touched = true;
          }
        }
        if (touched) store.itemsRev++;
      }
      // 目标完成/预算耗尽由服务端通过 thread/goal/updated 通知，回合完成不再自动清目标
      // 计划模式：回合正常完成且产出 plan 内容 → 弹出“计划已就绪”确认（仿 VS Code/CLI，
      // 纯客户端 UX：协议层没有计划确认交互，由客户端在计划 item 完成后自行询问）
      if (
        tab &&
        store.taskMode === "plan" &&
        !interrupted &&
        tab.followupQueue.length === 0 &&
        p.turn?.id &&
        tid &&
        tab.planPrompt?.turnId !== p.turn.id
      ) {
        const threadItems = store.itemsByThread[tid] ?? [];
        let planText = "";
        for (let i = threadItems.length - 1; i >= 0; i--) {
          const it = threadItems[i];
          if (it?.type === "plan" && typeof it.text === "string" && it.text.trim()) {
            planText = it.text;
            break;
          }
        }
        if (planText) {
          if (isActive) {
            store.planPrompt = { threadId: tid, turnId: p.turn.id, planText };
            if (store.settings.sound_enabled) playNotificationSound();
          } else {
            tab.planPrompt = { threadId: tid, turnId: p.turn.id, planText };
          }
        }
      }
      await refreshThreads();
      // 处理“加入队列”的跟进消息
      if (tab && tab.followupQueue.length) {
        const next = tab.followupQueue.shift()!;
        if (store.activeSessionId === tab.id) {
          // 活动标签：走 live 字段路径（发送状态落到当前会话）
          await continueTurn(next.text, next.attachments);
        } else if (tab.threadId) {
          // 后台标签：状态写入标签记录，不触碰活动标签
          await continueTurnForTab(tab, next.text, next.attachments);
        }
      }
    }),
  );

  unlisteners.push(
    await listen("item/started", (e) => {
      const p = e.payload as {
        item: ThreadItem;
        threadId: string;
        startedAtMs?: number;
      };
      if (isBackgroundThread(p.threadId)) return;
      upsertItem(p.threadId, {
        ...p.item,
        startedAtMs: p.startedAtMs ?? Date.now(),
      });
    }),
    await listen("item/completed", (e) => {
      const p = e.payload as {
        item: ThreadItem;
        threadId: string;
        completedAtMs?: number;
      };
      if (isBackgroundThread(p.threadId)) return;
      // 优先用服务端提供的耗时；缺失时用 startedAtMs→completedAtMs 推算，
      // 覆盖命令执行/文件变更等所有工具类型的“耗时”展示。
      let durationMs: number | undefined =
        typeof p.item.durationMs === "number" ? p.item.durationMs : undefined;
      if (durationMs === undefined) {
        const existing = findItem(p.threadId, p.item.id);
        const started = existing?.startedAtMs;
        if (
          typeof started === "number" &&
          typeof p.completedAtMs === "number" &&
          p.completedAtMs >= started
        ) {
          durationMs = p.completedAtMs - started;
        }
      }
      upsertItem(p.threadId, {
        ...p.item,
        completedAtMs: p.completedAtMs,
        streaming: false,
        ...(durationMs !== undefined ? { durationMs } : {}),
      });
    }),
  );

  unlisteners.push(
    await listen("item/agentMessage/delta", (e) => {
      const p = e.payload as { threadId: string; itemId: string; delta: string };
      if (isBackgroundThread(p.threadId)) return;
      let item = findItem(p.threadId, p.itemId);
      if (!item) {
        item = { id: p.itemId, type: "agentMessage", text: "", streaming: true };
        upsertItem(p.threadId, item);
      }
      item.text = (item.text ?? "") + p.delta;
      if (!item.streaming) {
        item.streaming = true;
        bumpActive(p.threadId, 1);
      }
      store.itemsRev++;
    }),
  );

  unlisteners.push(
    await listen("item/commandExecution/outputDelta", (e) => {
      const p = e.payload as { threadId: string; itemId: string; delta: string };
      if (isBackgroundThread(p.threadId)) return;
      let item = findItem(p.threadId, p.itemId);
      if (!item) {
        item = { id: p.itemId, type: "commandExecution", command: "", status: "in_progress", aggregatedOutput: "" };
        upsertItem(p.threadId, item);
      }
      item.aggregatedOutput = (item.aggregatedOutput ?? "") + p.delta;
      store.itemsRev++;
    }),
  );

  unlisteners.push(
    await listen("item/reasoning/textDelta", (e) => {
      const p = e.payload as { threadId: string; itemId: string; delta: string; contentIndex: number };
      if (isBackgroundThread(p.threadId)) return;
      let item = findItem(p.threadId, p.itemId);
      if (!item) {
        item = { id: p.itemId, type: "reasoning", content: [] };
        upsertItem(p.threadId, item);
      }
      if (typeof item.startedAtMs !== "number") {
        item.startedAtMs = Date.now();
      }
      if (!item.streaming) {
        item.streaming = true;
        bumpActive(p.threadId, 1);
      }
      const content = (item.content as string[] | undefined) ?? [];
      const idx = p.contentIndex ?? content.length - 1;
      if (idx >= 0 && idx < content.length) {
        content[idx] = (content[idx] ?? "") + p.delta;
      } else {
        content.push(p.delta);
      }
      item.content = content;
      store.itemsRev++;
    }),
  );

  unlisteners.push(
    await listen("item/fileChange/patchUpdated", (e) => {
      const p = e.payload as {
        threadId: string;
        itemId: string;
        changes?: { path: string; kind: string; diff?: string }[];
      };
      if (isBackgroundThread(p.threadId)) return;
      upsertItem(p.threadId, {
        id: p.itemId,
        type: "fileChange",
        changes: p.changes ?? [],
        status: "inProgress",
        streaming: true,
      });
    }),
  );

  unlisteners.push(
    await listen("thread/name/updated", (e) => {
      const p = e.payload as { threadId: string; threadName?: string };
      const t = store.threads.find((x) => x.id === p.threadId);
      if (t) t.name = p.threadName ?? null;
      const tab = findSessionTabByThread(p.threadId);
      if (tab && p.threadName) {
        tab.name = p.threadName;
        tab.title = sessionTabTitle(tab);
        if (store.activeSessionId === tab.id) {
          store.currentThreadName = p.threadName;
        }
      }
    }),
    await listen("thread/tokenUsage/updated", (e) => {
      const p = e.payload as {
        threadId: string;
        tokenUsage?: {
          total?: { totalTokens?: number };
          last?: { totalTokens?: number };
          modelContextWindow?: number | null;
        };
      };
      const usage = {
        // 当前上下文占用取 last（最近一次请求），total 为会话累计（会超过窗口）
        used:
          p.tokenUsage?.last?.totalTokens ??
          p.tokenUsage?.total?.totalTokens ??
          0,
        window: p.tokenUsage?.modelContextWindow ?? null,
      };
      const tab = findSessionTabByThread(p.threadId);
      if (tab && store.activeSessionId === tab.id) {
        store.threadTokenUsage = usage;
      } else if (tab) {
        tab.threadTokenUsage = usage;
      } else if (p.threadId === store.currentThreadId) {
        store.threadTokenUsage = {
          ...usage,
        };
      }
    }),
    await listen("thread/status/changed", (e) => {
      const p = e.payload as { threadId: string; status: { type: string } };
      const t = store.threads.find((x) => x.id === p.threadId);
      if (t) t.status = p.status;
    }),
    await listen("thread/goal/updated", (e) => {
      const p = e.payload as {
        threadId?: string;
        goal?: { objective?: string; status?: string };
      };
      if (isBackgroundThread(p.threadId)) return;
      const tab = p.threadId
        ? findSessionTabByThread(p.threadId)
        : activeSessionTab();
      if (!tab) return; // 未打开的线程目标不进入 UI
      const isActive = store.activeSessionId === tab.id;
      if (p.goal) {
        const objective =
          typeof p.goal.objective === "string" ? p.goal.objective : null;
        const status = isGoalStatus(p.goal.status) ? p.goal.status : null;
        if (isActive) {
          store.goalText = objective;
          store.goalStatus = status;
        } else {
          tab.goalText = objective;
          tab.goalStatus = status;
        }
        // 服务端终态：目标已完成/预算耗尽/受限/阻塞/暂停 → toast 提示并复位
        // （先同步清本地，视觉上即“无目标”；后台 goal_clear 同步服务端）
        if (isGoalTerminalStatus(p.goal.status)) {
          setToast(goalStatusToast(p.goal.status));
          if (isActive) {
            store.goalText = null;
            store.goalStatus = null;
            store.goalArmed = false;
          } else {
            tab.goalText = null;
            tab.goalStatus = null;
            tab.goalArmed = false;
          }
          void clearGoal(tab.threadId ?? store.currentThreadId);
        }
      }
    }),
    await listen("thread/goal/cleared", (e) => {
      const p = e.payload as { threadId?: string };
      if (isBackgroundThread(p.threadId)) return;
      const tab = p.threadId
        ? findSessionTabByThread(p.threadId)
        : activeSessionTab();
      if (!tab) return;
      if (store.activeSessionId === tab.id) {
        store.goalText = null;
        store.goalStatus = null;
        store.goalArmed = false;
      } else {
        tab.goalText = null;
        tab.goalStatus = null;
        tab.goalArmed = false;
      }
    }),
    await listen("thread/started", (e) => {
      const p = e.payload as { thread?: { id?: string } };
      if (isBackgroundThread(p?.thread?.id)) return; // 临时线程不触发历史刷新
      void refreshThreads();
    }),
  );

  unlisteners.push(
    await listen("error", (e) => {
      const p = e.payload as {
        error?: { message?: string; codexErrorInfo?: unknown };
      };
      const err = p.error;
      setToast(
        err && (err.message || err.codexErrorInfo)
          ? friendlyServerError(err)
          : "codex 发生错误",
      );
    }),
    await listen("warning", (e) => {
      const p = e.payload as { message?: string };
      if (p?.message) setToast(friendlyServerMessage(p.message));
    }),
  );
}

/** 启动加载态最长展示时长：防止某个 invoke 挂起导致加载动画永久显示 */
const BOOT_MAX_MS = 15_000;

export async function init() {
  const bootTimer = window.setTimeout(() => {
    store.booting = false;
  }, BOOT_MAX_MS);
  try {
    // 先拿到工作目录：沙箱可写根与资源/Git 面板需要它；历史列表有意展示全部目录的会话。
    await Promise.all([loadSettings(), refreshServer()]);
    // 主窗口标题固定为 “Codex UI”，与当前会话无关（非 Tauri 环境静默忽略）
    try {
      await getCurrentWindow().setTitle("Codex UI");
    } catch {
      // 忽略非 Tauri 环境
    }
    void loadModels();
    void ensureThreadPlugins(NEW_CHAT_PLUGIN_KEY); // 应用启动预初始化新对话插件缓存
    void ensureSkills(); // 应用启动预加载技能列表（$ 菜单与回显悬浮提示共用）
    await refreshThreads();
    await wireEvents();
    // 监听注册后补取一次状态：避免后端启动成功的首次推送早于监听注册被丢弃
    void refreshServer().catch(() => undefined);
  } finally {
    window.clearTimeout(bootTimer);
    store.booting = false;
  }
}

export function disposeEvents() {
  for (const fn of unlisteners) {
    try {
      fn();
    } catch {
      // ignore
    }
  }
  unlisteners = [];
  wired = false;
}

export function permissionChip(): string {
  return permissionMode(store.permissionMode).chip;
}

export function currentItems(): ThreadItem[] {
  return store.currentThreadId ? (store.itemsByThread[store.currentThreadId] ?? []) : [];
}

export function threadTitle(t: ThreadSummary): string {
  return t.name || t.preview || "新会话";
}

export function currentOriginLabel(): string {
  if (store.currentThreadOrigin === "history") return "历史会话";
  if (store.currentThreadOrigin === "new") return "新会话";
  return "新会话";
}
