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
import { playNotificationSound } from "../lib/sound";

const defaultSettings = (): AppSettings => ({
  permission_mode: "ask-for-approval",
  model: null,
  effort: null,
  codex_path: null,
  sound_enabled: true,
});

export const store = reactive({
  server: {
    connected: false,
    workspace: "",
    codexPath: null as string | null,
    logs: [] as string[],
  },
  threads: [] as ThreadSummary[],
  threadsTotal: 0,
  nextCursor: null as string | null,
  currentThreadId: null as string | null,
  currentThreadName: "",
  currentThreadOrigin: null as "new" | "history" | null,
  currentThreadCwd: null as string | null,
  resumedThreadId: null as string | null,
  itemsByThread: {} as Record<string, ThreadItem[]>,
  turnActive: false,
  turnInterrupted: false,
  currentTurnId: null as string | null,
  interactions: [] as PendingInteraction[],
  settings: defaultSettings(),
  loadingHistory: false,
  busy: false,
  currentModel: "",
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

function upsertItem(threadId: string, item: ThreadItem) {
  const arr = (store.itemsByThread[threadId] ??= []);
  let idx = arr.findIndex((x) => x.id === item.id);
  if (idx < 0 && item.type === "userMessage" && item.clientId) {
    // 服务端推送的用户消息与本地乐观插入的通过 clientId 关联，避免重复
    idx = arr.findIndex(
      (x) => x.type === "userMessage" && x.clientId === item.clientId,
    );
  }
  if (idx >= 0) {
    arr[idx] = { ...arr[idx], ...item };
  } else {
    arr.push(item);
  }
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
    await win.setTitle(`codex-ui · ${currentThreadLabel()}`);
  } catch {
    // 非 Tauri 环境（如浏览器预览）忽略
  }
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

export async function refreshThreads(loadMore = false) {
  if (store.loadingHistory) return;
  store.loadingHistory = true;
  try {
    const res = await invoke<{
      data: ThreadSummary[];
      nextCursor: string | null;
    }>("thread_list", {
      limit: loadMore ? 50 : 30,
      cursor: loadMore ? store.nextCursor : null,
    });
    store.threads = loadMore ? [...store.threads, ...res.data] : res.data;
    store.nextCursor = res.nextCursor;
    store.threadsTotal = store.threads.length;
  } catch (e) {
    setToast(String(e));
  } finally {
    store.loadingHistory = false;
  }
}

async function newChat(prompt: string, attachments: UserInput[]) {
  store.busy = true;
  try {
    const params: Record<string, unknown> = {
      cwd: store.server.workspace,
      approvalPolicy: toApprovalPolicy(store.settings.permission_mode),
      sandbox: toSandbox(store.settings.permission_mode),
    };
    const reviewer = toApprovalsReviewer(store.settings.permission_mode);
    if (reviewer) params.approvalsReviewer = reviewer;
    if (store.settings.model) params.model = store.settings.model;
    const res = await invoke<{ thread: { id: string; name?: string | null }; model?: string }>(
      "thread_start",
      { params },
    );
    const threadId = res.thread.id;
    store.currentThreadId = threadId;
    store.currentThreadName = res.thread.name ?? "";
    store.currentThreadOrigin = "new";
    store.currentThreadCwd = store.server.workspace;
    store.resumedThreadId = threadId;
    store.currentModel = res.model ?? store.settings.model ?? "";
    store.itemsByThread[threadId] = [];
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
    await updateWindowTitle();
    if (prompt.trim() || attachments.length) {
      await continueTurn(prompt, attachments);
    }
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
      setToast(String(e));
      return;
    }
  }
  const clientId = `user-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const input: UserInput[] = [
    { type: "text", text: prompt, text_elements: [] },
    ...attachments,
  ];
  const params: Record<string, unknown> = { threadId, input, clientUserMessageId: clientId };
  // 权限模式随每一轮发送（协议：本回合及后续回合生效），空闲期切换后立即生效
  params.approvalPolicy = toApprovalPolicy(store.settings.permission_mode);
  params.sandboxPolicy = toSandboxPolicy(
    store.settings.permission_mode,
    store.currentThreadCwd ?? store.server.workspace,
  );
  const reviewer = toApprovalsReviewer(store.settings.permission_mode);
  if (reviewer) params.approvalsReviewer = reviewer;
  if (store.settings.model) params.model = store.settings.model;
  if (store.settings.effort) params.effort = store.settings.effort;
  if (store.taskMode === "plan") {
    params.collaborationMode = {
      mode: "plan",
      settings: {
        model: store.settings.model ?? store.currentModel ?? "",
        reasoning_effort: store.settings.effort ?? null,
        developer_instructions: null,
      },
    };
  }
  upsertItem(threadId, {
    id: clientId,
    clientId,
    type: "userMessage",
    content: input,
  });
  try {
    const res = await invoke<{ turn?: { id?: string } }>("turn_start", { params });
    store.turnActive = true;
    // 立即记录回合 id，供 turn/interrupt 使用（turn/started 事件可能稍后才到）
    if (res?.turn?.id) store.currentTurnId = res.turn.id;
  } catch (e) {
    setToast(String(e));
    store.turnActive = false;
  }
}

export async function sendPrompt(text: string) {
  const attachments = store.attachments.splice(0);
  if (!text.trim() && attachments.length === 0) return;
  try {
    if (!store.currentThreadId) {
      await newChat(text, attachments);
    } else {
      await continueTurn(text, attachments);
    }
  } catch (e) {
    setToast(String(e));
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
  store.showHistory = false;
  store.goalText = null;
  await updateWindowTitle();
}

export async function openThread(threadId: string) {
  store.currentThreadId = threadId;
  store.currentThreadOrigin = "history";
  store.showHistory = false;
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
    store.itemsByThread[threadId] = flattenTurns(res.thread.turns);
    store.currentThreadName = res.thread.name ?? "";
    store.currentThreadCwd = res.thread.cwd ?? null;
    store.resumedThreadId = null; // 只读打开，不恢复；发消息时才恢复
    store.turnActive = false;
    store.turnInterrupted = false;
    store.currentTurnId = null;
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
    setToast(String(e));
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
      let durationMs: number | undefined;
      if (p.item.type === "reasoning") {
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
        durationMs,
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
      item.streaming = true;
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
      item.streaming = true;
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
  return permissionMode(store.settings.permission_mode).chip;
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
        .map((c) => (c.type === "text" ? c.text : ""))
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
