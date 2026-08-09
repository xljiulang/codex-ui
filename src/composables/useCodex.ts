import { reactive, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type {
  AppSettings,
  PendingInteraction,
  ServerStatus,
  ThreadItem,
  ThreadSummary,
  Turn,
  UserInput,
} from "../lib/types";
import {
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

const defaultSettings = (): AppSettings => ({
  codex_path: null,
  sound_enabled: true,
  enter_to_send: true,
  followup_mode: "adjust",
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

export const store = reactive({
  server: {
    connected: false,
    workspace: "",
    codexPath: null as string | null,
    logs: [] as string[],
  },
  threads: [] as ThreadSummary[],
  nextCursor: null as string | null,
  searchActive: false,
  searchSnippets: {} as Record<string, string>,
  searchCursor: null as string | null,
  currentThreadId: null as string | null,
  currentThreadName: "",
  currentThreadOrigin: null as "new" | "history" | null,
  currentThreadCwd: null as string | null,
  resumedThreadId: null as string | null,
  itemsByThread: {} as Record<string, ThreadItem[]>,
  // 每个线程“进行中工作”计数（流式文本/进行中工具），避免渲染时全量扫描
  activeWorkByThread: {} as Record<string, number>,
  turnActive: false,
  turnInterrupted: false,
  currentTurnId: null as string | null,
  followupQueue: [] as { text: string; attachments: UserInput[] }[],
  interactions: [] as PendingInteraction[],
  settings: defaultSettings(),
  loadingHistory: false,
  loadingThread: false,
  busy: false,
  currentModel: "",
  // 进程级设置：权限模式 / 模型 / 推理强度，仅当前运行期有效，不写入配置文件
  permissionMode: "ask-for-approval" as string,
  model: null as string | null,
  effort: null as string | null,
  models: [] as ModelInfo[],
  modelsLoaded: false,
  threadTokenUsage: null as { used: number; window: number | null } | null,
  // 新建对话时可选的项目目录（null = 使用启动工作目录）
  newChatCwd: null as string | null,
  taskMode: "execute" as "execute" | "plan" | "goal",
  goalText: null as string | null,
  attachments: [] as UserInput[],
  showHistory: false,
  showSettings: false,
  plusOpen: false,
  permOpen: false,
  taskOpen: false,
  modelOpen: false,
  goalOpen: false,
  toast: "",
});

let unlisteners: UnlistenFn[] = [];
let wired = false;

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

function setToast(msg: string) {
  store.toast = msg;
}

/** 把错误对象转成可读提示：优先取 error.error.message / error.message，避免显示原始 JSON */
export function toastError(e: unknown): string {
  if (e && typeof e === "object") {
    const err = e as { error?: { message?: unknown }; message?: unknown };
    if (typeof err.error?.message === "string") return err.error.message;
    if (typeof err.message === "string") return err.message;
  }
  return String(e);
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
          sortDirection: "ascending",
          itemsView: "full",
        },
      })) as TurnsListPage;
      turns.push(...(res.data ?? []));
      cursor = res.nextCursor ?? null;
      if (!cursor) break;
      if (turns.length > 2000) break; // 防超长会话
    }
  } catch {
    return null; // 服务端不支持时回退到 thread_read 的摘要项
  }
  return flattenTurns(turns);
}

async function focusWindow() {
  try {
    const win = getCurrentWindow();
    const minimized = await win.isMinimized();
    if (minimized) await win.unminimize();
    await win.setFocus();
  } catch {
    // 非 Tauri 环境（如浏览器预览）忽略
  }
}

async function updateWindowTitle() {
  try {
    const win = getCurrentWindow();
    if (!store.currentThreadId) {
      await win.setTitle("codex-ui");
      return;
    }
    await win.setTitle(currentThreadLabel());
  } catch {
    // 非 Tauri 环境（如浏览器预览）忽略
  }
}

function isThreadNotFound(e: unknown): boolean {
  return String(e).toLowerCase().includes("thread not found");
}

/** 当前会话已不存在（被删除等）时重置回新对话，避免继续发送一直报错 */
function resetToNewChat() {
  store.currentThreadId = null;
  store.currentThreadName = "";
  store.currentThreadOrigin = null;
  store.currentThreadCwd = null;
  store.resumedThreadId = null;
  store.currentTurnId = null;
  store.turnActive = false;
  store.turnInterrupted = false;
  void updateWindowTitle();
}

export async function loadSettings() {
  try {
    const s = await invoke<AppSettings>("settings_get");
    store.settings = { ...defaultSettings(), ...s };
  } catch {
    store.settings = defaultSettings();
  }
}

export async function saveSettings(patch: Partial<AppSettings>) {
  store.settings = { ...store.settings, ...patch };
  await invoke("settings_set", { settings: store.settings });
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

/** 固定优先，再按最近时间降序 */
export function sortThreads(list: ThreadSummary[]): ThreadSummary[] {
  return [...list].sort((a, b) => {
    if (!!a.isPinned !== !!b.isPinned) return a.isPinned ? -1 : 1;
    const ar = a.recencyAt ?? a.updatedAt ?? 0;
    const br = b.recencyAt ?? b.updatedAt ?? 0;
    return br - ar;
  });
}

export async function refreshThreads(loadMore = false) {
  if (store.loadingHistory) return;
  store.loadingHistory = true;
  try {
    const res = await invoke<{
      data: ThreadSummary[];
      nextCursor: string | null;
    }>("thread_list", {
      limit: 50,
      cursor: loadMore ? store.nextCursor : null,
    });
    store.threads = sortThreads(
      loadMore ? [...store.threads, ...res.data] : res.data,
    );
    store.nextCursor = res.nextCursor;
  } catch (e) {
    setToast(String(e));
  } finally {
    store.loadingHistory = false;
  }
}

/** 搜索历史会话（thread/search），结果写入 store.threads 并附带摘要 */
export async function searchThreads(term: string, loadMore = false) {
  const t = term.trim();
  if (!t) {
    clearSearch();
    return;
  }
  if (store.loadingHistory) return;
  store.loadingHistory = true;
  try {
    const res = await invoke<{
      data: { thread: ThreadSummary; snippet: string }[];
      nextCursor: string | null;
    }>("codex_rpc", {
      method: "thread/search",
      params: {
        searchTerm: t,
        limit: 50,
        cursor: loadMore ? store.searchCursor : null,
        sourceKinds: ["cli", "vscode", "exec", "appServer", "unknown"],
      },
    });
    const results = res.data ?? [];
    const snippets: Record<string, string> = {};
    for (const r of results) snippets[r.thread.id] = r.snippet ?? "";
    store.searchSnippets = loadMore
      ? { ...store.searchSnippets, ...snippets }
      : snippets;
    store.threads = sortThreads(
      loadMore
        ? [...store.threads, ...results.map((r) => r.thread)]
        : results.map((r) => r.thread),
    );
    store.searchCursor = res.nextCursor ?? null;
    store.searchActive = true;
  } catch (e) {
    setToast(String(e));
  } finally {
    store.loadingHistory = false;
  }
}

/** 退出搜索，恢复常规列表 */
export function clearSearch() {
  store.searchActive = false;
  store.searchSnippets = {};
  store.searchCursor = null;
  void refreshThreads();
}

/** 重命名会话 */
export async function renameThread(threadId: string, name: string) {
  const n = name.trim();
  if (!n) return;
  try {
    await invoke("thread_set_name", { threadId, name: n });
    const t = store.threads.find((x) => x.id === threadId);
    if (t) t.name = n;
    if (store.currentThreadId === threadId) {
      store.currentThreadName = n;
      void updateWindowTitle();
    }
  } catch (e) {
    setToast(String(e));
  }
}

/** 固定/取消固定会话（置顶） */
export async function togglePin(threadId: string, pinned: boolean) {
  try {
    await invoke("codex_rpc", {
      method: "thread/metadata/update",
      params: { threadId, isPinned: pinned },
    });
    await refreshThreads();
  } catch (e) {
    setToast(String(e));
  }
}

async function newChat(prompt: string, attachments: UserInput[]) {
  store.busy = true;
  try {
    const cwd = store.newChatCwd ?? store.server.workspace;
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
    store.currentThreadId = threadId;
    store.currentThreadName = res.thread.name ?? "";
    store.currentThreadOrigin = "new";
    store.currentThreadCwd = cwd;
    store.resumedThreadId = threadId;
    store.newChatCwd = null; // 本次新建已消费，恢复默认
    store.currentModel = res.model ?? currentModelId();
    store.itemsByThread[threadId] = [];
    store.activeWorkByThread[threadId] = 0;
    store.showHistory = false;
    const pendingGoal = store.goalText;
    if (pendingGoal) {
      try {
        await invoke("goal_set", { threadId, objective: pendingGoal });
      } catch {
        // 目标设置失败不阻塞新建会话
      }
    }
    store.goalText = pendingGoal;
    await refreshThreads();
    if (prompt.trim() || attachments.length) {
      await continueTurn(prompt, attachments);
    }
    await updateWindowTitle();
  } finally {
    store.busy = false;
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
        setToast("会话已不存在，已切换为新对话");
      } else {
        setToast(toastError(e));
      }
      return;
    }
  }
  const clientId = `user-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  // 与 VS Code Codex 扩展一致：文件引用序列化成文本段落，作为单条 text 输入
  const input = buildTurnInput(prompt, attachments);
  const params: Record<string, unknown> = { threadId, input, clientUserMessageId: clientId };
  // 权限模式随每一轮发送（协议：本回合及后续回合生效），空闲期切换后立即生效
  params.approvalPolicy = toApprovalPolicy(store.permissionMode);
  params.sandboxPolicy = toSandboxPolicy(
    store.permissionMode,
    store.currentThreadCwd ?? store.server.workspace,
  );
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
  upsertItem(threadId, {
    id: clientId,
    clientId,
    type: "userMessage",
    content: input,
    startedAtMs: Date.now(),
  });
  await updateWindowTitle(); // 首条消息发送后窗口标题立即跟随
  try {
    const res = await invoke<{ turn?: { id?: string } }>("turn_start", { params });
    store.turnActive = true;
    // 立即记录回合 id，供 turn/interrupt 使用（turn/started 事件可能稍后才到）
    if (res?.turn?.id) store.currentTurnId = res.turn.id;
  } catch (e) {
    if (isThreadNotFound(e)) {
      resetToNewChat();
      setToast("会话已不存在，已切换为新对话");
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

export async function newEmptyChat() {
  store.currentThreadId = null;
  store.currentThreadName = "";
  store.currentThreadOrigin = null;
  store.currentThreadCwd = null;
  store.resumedThreadId = null;
  store.turnInterrupted = false;
  store.currentTurnId = null;
  store.threadTokenUsage = null;
  store.showHistory = false;
  store.goalText = null;
  await updateWindowTitle();
}

export async function openThread(threadId: string) {
  store.currentThreadId = threadId;
  store.currentThreadOrigin = "history";
  store.showHistory = false;
  store.loadingThread = true;
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
    store.currentThreadName = res.thread.name ?? "";
    store.currentThreadCwd = res.thread.cwd ?? null;
    store.resumedThreadId = null; // 只读打开，不恢复；发消息时才恢复
    store.turnActive = false;
    store.turnInterrupted = false;
    store.currentTurnId = null;
    store.threadTokenUsage = null;
    try {
      const g = await invoke<{ objective?: string; goal?: { objective?: string } }>(
        "goal_get",
        { threadId },
      );
      store.goalText = g?.objective ?? g?.goal?.objective ?? null;
    } catch {
      store.goalText = null;
    }
    await updateWindowTitle();
  } catch (e) {
    if (isThreadNotFound(e)) {
      resetToNewChat();
      setToast("会话已不存在，已切换为新对话");
    } else {
      setToast(String(e));
    }
  } finally {
    store.loadingThread = false;
  }
}

export async function interrupt() {
  if (!store.currentThreadId) return;
  if (!store.currentTurnId) return;
  // 服务端可能在 turn/started 事件之后才把回合标记为 active；
  // 若用户点得过早会收到 “no active turn”，短暂重试几次。
  for (let attempt = 0; attempt < 5; attempt++) {
    try {
      await invoke("turn_interrupt", {
        threadId: store.currentThreadId,
        turnId: store.currentTurnId,
      });
      return;
    } catch (e) {
      const msg = String(e);
      if (msg.includes("no active turn")) {
        await new Promise((r) => setTimeout(r, 400));
        continue;
      }
      setToast(msg);
      return;
    }
  }
}

export async function setGoal(objective: string) {
  if (!store.currentThreadId) {
    store.goalText = objective;
    store.goalOpen = false;
    return;
  }
  try {
    await invoke("goal_set", { threadId: store.currentThreadId, objective });
    store.goalText = objective;
    store.goalOpen = false;
  } catch (e) {
    setToast(String(e));
  }
}

export async function clearGoal() {
  if (!store.currentThreadId) {
    store.goalText = null;
    if (store.taskMode === "goal") store.taskMode = "execute";
    return;
  }
  try {
    await invoke("goal_clear", { threadId: store.currentThreadId });
    store.goalText = null;
    if (store.taskMode === "goal") {
      store.taskMode = "execute";
    }
  } catch (e) {
    setToast(String(e));
  }
}

export async function deleteThread(threadId: string) {
  try {
    await invoke("thread_delete", { threadId });
    store.threads = store.threads.filter((t) => t.id !== threadId);
    if (store.currentThreadId === threadId) {
      store.currentThreadId = null;
      store.currentThreadName = "";
      store.currentThreadCwd = null;
      store.resumedThreadId = null;
      store.threadTokenUsage = null;
      await updateWindowTitle();
    }
  } catch (e) {
    setToast(String(e));
  }
}

export async function respondInteraction(interaction: PendingInteraction, result: unknown) {
  try {
    await invoke("interaction_respond", {
      requestId: interaction.requestId,
      result,
    });
  } catch (e) {
    setToast(String(e));
  } finally {
    store.interactions = store.interactions.filter((i) => i.requestId !== interaction.requestId);
  }
}

async function wireEvents() {
  if (wired) return;
  wired = true;

  unlisteners.push(
    await listen("server/status", (e) => {
      store.server = { ...store.server, ...(e.payload as ServerStatus) };
    }),
  );

  unlisteners.push(
    await listen("interaction:request", async (e) => {
      const p = e.payload as { requestId: number; method: string; params: Record<string, unknown> };
      store.interactions.push({ ...p, at: Date.now() });
      if (store.settings.sound_enabled) playNotificationSound();
      await focusWindow();
    }),
  );

  unlisteners.push(
    await listen("serverRequest/resolved", (e) => {
      const p = e.payload as { requestId: number };
      store.interactions = store.interactions.filter((i) => i.requestId !== p.requestId);
    }),
  );

  unlisteners.push(
    await listen("turn/started", (e) => {
      const p = e.payload as { turn?: { id?: string } };
      store.turnActive = true;
      store.turnInterrupted = false;
      if (p.turn?.id) store.currentTurnId = p.turn.id;
    }),
    await listen("turn/completed", async (e) => {
      const p = e.payload as { turn?: { status?: string } };
      store.turnActive = false;
      store.turnInterrupted = p.turn?.status === "interrupted";
      store.currentTurnId = null;
      await refreshThreads();
      // 处理“加入队列”的跟进消息
      if (store.followupQueue.length) {
        const next = store.followupQueue.shift()!;
        if (store.currentThreadId) {
          await continueTurn(next.text, next.attachments);
        } else {
          await newChat(next.text, next.attachments);
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
    }),
  );

  unlisteners.push(
    await listen("item/commandExecution/outputDelta", (e) => {
      const p = e.payload as { threadId: string; itemId: string; delta: string };
      let item = findItem(p.threadId, p.itemId);
      if (!item) {
        item = { id: p.itemId, type: "commandExecution", command: "", status: "in_progress", aggregatedOutput: "" };
        upsertItem(p.threadId, item);
      }
      item.aggregatedOutput = (item.aggregatedOutput ?? "") + p.delta;
    }),
  );

  unlisteners.push(
    await listen("item/reasoning/textDelta", (e) => {
      const p = e.payload as { threadId: string; itemId: string; delta: string; contentIndex: number };
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
    }),
  );

  unlisteners.push(
    await listen("item/fileChange/patchUpdated", (e) => {
      const p = e.payload as {
        threadId: string;
        itemId: string;
        changes?: { path: string; kind: string; diff?: string }[];
      };
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
      if (store.currentThreadId === p.threadId && p.threadName) {
        store.currentThreadName = p.threadName;
        void updateWindowTitle();
      }
    }),
    await listen("thread/tokenUsage/updated", (e) => {
      const p = e.payload as {
        threadId: string;
        tokenUsage?: {
          total?: { totalTokens?: number };
          modelContextWindow?: number | null;
        };
      };
      if (p.threadId === store.currentThreadId) {
        store.threadTokenUsage = {
          used: p.tokenUsage?.total?.totalTokens ?? 0,
          window: p.tokenUsage?.modelContextWindow ?? null,
        };
      }
    }),
    await listen("thread/status/changed", (e) => {
      const p = e.payload as { threadId: string; status: { type: string } };
      const t = store.threads.find((x) => x.id === p.threadId);
      if (t) t.status = p.status;
    }),
    await listen("thread/started", async () => {
      await refreshThreads();
    }),
  );

  unlisteners.push(
    await listen("error", (e) => {
      const p = e.payload as { error?: { message?: string } };
      setToast(p.error?.message ?? "codex 发生错误");
    }),
    await listen("warning", (e) => {
      const p = e.payload as { message?: string };
      if (p?.message) setToast(p.message);
    }),
  );
}

export async function init() {
  // 先拿到工作目录：窗口标题与沙箱可写根需要它；历史列表有意展示全部目录的会话。
  await Promise.all([loadSettings(), refreshServer()]);
  await updateWindowTitle();
  void loadModels();
  await refreshThreads();
  await wireEvents();
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
  return t.name || t.preview || "新对话";
}

export function currentThreadLabel(): string {
  if (!store.currentThreadId) return "新对话";
  if (store.currentThreadName) return store.currentThreadName;
  const summary = store.threads.find((t) => t.id === store.currentThreadId);
  if (summary?.name || summary?.preview) {
    return summary.name || summary.preview || "新对话";
  }
  const items = store.itemsByThread[store.currentThreadId] ?? [];
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.type === "userMessage") {
      const content = (it.content as UserInput[] | undefined) ?? [];
      const text = content
        .map((c) => (c.type === "text" ? stripMentionContext(c.text) : ""))
        .join(" ")
        .trim();
      if (text) return text;
    }
  }
  return "新对话";
}

export function currentOriginLabel(): string {
  if (store.currentThreadOrigin === "history") return "历史会话";
  if (store.currentThreadOrigin === "new") return "新会话";
  return "新对话";
}
