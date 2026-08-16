// useCodex 拆分模块：事件桥（原 useCodex.ts 的一部分，纯移动，行为不变）
import { watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, ProgressBarStatus } from "@tauri-apps/api/window";
import { friendlyServerError, friendlyServerMessage } from "../../lib/serverMessages";
import {
  isThreadItemType,
  type AgentMessageItem,
  type CommandExecutionItem,
  type ReasoningItem,
  type ServerStatus,
  type ThreadItem,
} from "../../lib/types";
import { playNotificationSound } from "../../lib/sound";
import {
  bumpActive,
  findItem,
  getOrCreateItem,
  isActiveItem,
  upsertItem,
} from "./items";
import { activeSessionTab, allSessionTabs, findSessionTabByThread, sessionTabTitle } from "./sessionState";
import { isBackgroundThread, store } from "./store";
import { refreshThreads } from "./threads";
import { setToast } from "./toast";
import { clearGoal, continueTurn, continueTurnForTab } from "./turnControl";
import {
  goalStatusToast,
  isGoalStatus,
  isGoalTerminalStatus,
  type SessionTab,
} from "./types";


let unlisteners: UnlistenFn[] = [];
let wired = false;


/** 回合进行中时在任务栏显示不确定进度条，结束后隐藏（非 Tauri 环境静默忽略） */
async function updateTaskbarProgress() {
  try {
    const win = getCurrentWindow();
    await win.setProgressBar({
      status: activeSessionTab()?.turnActive
        ? ProgressBarStatus.Indeterminate
        : ProgressBarStatus.None,
    });
  } catch {
    // 非 Tauri 环境（如浏览器预览）忽略
  }
}


watch(
  () => activeSessionTab()?.turnActive,
  () => void updateTaskbarProgress(),
  { immediate: true },
);

/**
 * 回合事件归属解析：事件必须能证明归属到某个会话标签——
 * 优先 `threadId`；缺 `threadId` 时按回合 id 匹配标签的 `currentTurnId`；
 * 仅当 turn id 与活动会话标签的 `currentTurnId` 一致时才回退活动会话；
 * 完全无法归因返回 null（调用方跳过）。禁止把缺失身份的事件默认当活动会话处理。
 */
export function resolveTurnTab(p: {
  threadId?: string;
  turn?: { id?: string };
}): SessionTab | null {
  if (p.threadId) return findSessionTabByThread(p.threadId) ?? null;
  const turnId = p.turn?.id;
  if (turnId) {
    const byTurn = allSessionTabs().find((t) => t.currentTurnId === turnId);
    if (byTurn) return byTurn;
    if (activeSessionTab()?.currentTurnId === turnId) return activeSessionTab();
  }
  return null;
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
      for (const tab of allSessionTabs()) {
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
      const tab = resolveTurnTab(p);
      if (!tab) return;
      // 回合状态一律写归属标签（tab 是唯一事实源，不再写 store）
      tab.turnActive = true;
      tab.turnInterrupted = false;
      tab.planPrompt = null; // 新回合开始：关闭“计划已就绪”确认弹窗
      // currentTurnId 保留 turn/start 响应的服务端回合 id（turn_interrupt 需要）；
      // 事件 id 仅在响应缺失时兜底。
      if (!tab.currentTurnId && p.turn?.id) tab.currentTurnId = p.turn.id;
    }),
    await listen("turn/completed", async (e) => {
      const p = e.payload as {
        threadId?: string;
        turn?: { id?: string; status?: string };
      };
      if (isBackgroundThread(p.threadId)) return; // 后台临时线程完成不影响主对话
      const tab = resolveTurnTab(p);
      // 无打开标签时（线程已关闭/仅缓存）仍按 p.threadId 清扫该线程的 item，
      // 但不触碰任何标签的回合状态/计划提示
      const tid = p.threadId ?? tab?.threadId;
      const interrupted = p.turn?.status === "interrupted";
      if (tab) {
        // 回合状态一律写归属标签（tab 是唯一事实源，不再写 store）
        tab.turnActive = false;
        tab.turnInterrupted = interrupted;
        tab.currentTurnId = null;
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
        tab.taskMode === "plan" &&
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
          tab.planPrompt = { threadId: tid, turnId: p.turn.id, planText };
          if (store.settings.sound_enabled) playNotificationSound();
        }
      }
      await refreshThreads();
      // 处理“加入队列”的跟进消息
      if (tab && tab.followupQueue.length) {
        const next = tab.followupQueue.shift()!;
        if (activeSessionTab()?.id === tab.id) {
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
      const item = getOrCreateItem(p.threadId, p.itemId, () => ({
        id: p.itemId,
        type: "agentMessage",
        text: "",
        streaming: true,
      }));
      if (!isThreadItemType<AgentMessageItem>(item, "agentMessage")) return;
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
      const item = getOrCreateItem(p.threadId, p.itemId, () => ({
        id: p.itemId,
        type: "commandExecution",
        command: "",
        status: "in_progress",
        aggregatedOutput: "",
      }));
      if (!isThreadItemType<CommandExecutionItem>(item, "commandExecution"))
        return;
      item.aggregatedOutput = (item.aggregatedOutput ?? "") + p.delta;
      store.itemsRev++;
    }),
  );

  unlisteners.push(
    await listen("item/reasoning/textDelta", (e) => {
      const p = e.payload as { threadId: string; itemId: string; delta: string; contentIndex: number };
      if (isBackgroundThread(p.threadId)) return;
      const item = getOrCreateItem(p.threadId, p.itemId, () => ({
        id: p.itemId,
        type: "reasoning",
        content: [],
      }));
      if (!isThreadItemType<ReasoningItem>(item, "reasoning")) return;
      if (typeof item.startedAtMs !== "number") {
        item.startedAtMs = Date.now();
      }
      if (!item.streaming) {
        item.streaming = true;
        bumpActive(p.threadId, 1);
      }
      const content = item.content ?? [];
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
      // token 用量一律写归属标签；未打开线程的事件无处可写 → 跳过
      if (tab) tab.threadTokenUsage = usage;
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
      // 目标事件无回合 id 可归因：缺 threadId 时跳过，不回退活动会话
      const tab = p.threadId ? findSessionTabByThread(p.threadId) : null;
      if (!tab) return; // 未打开的线程目标不进入 UI
      if (p.goal) {
        const objective =
          typeof p.goal.objective === "string" ? p.goal.objective : null;
        const status = isGoalStatus(p.goal.status) ? p.goal.status : null;
        // 目标状态一律写归属标签（tab 是唯一事实源，不再写 store）
        tab.goalText = objective;
        tab.goalStatus = status;
        // 服务端终态：目标已完成/预算耗尽/受限/阻塞/暂停 → toast 提示并复位
        // （先同步清本地，视觉上即“无目标”；后台 goal_clear 同步服务端）
        if (isGoalTerminalStatus(p.goal.status)) {
          setToast(goalStatusToast(p.goal.status));
          tab.goalText = null;
          tab.goalStatus = null;
          tab.goalArmed = false;
          void clearGoal(tab.threadId);
        }
      }
    }),
    await listen("thread/goal/cleared", (e) => {
      const p = e.payload as { threadId?: string };
      if (isBackgroundThread(p.threadId)) return;
      // 缺线程身份无法归因：跳过，不回退活动会话
      const tab = p.threadId ? findSessionTabByThread(p.threadId) : null;
      if (!tab) return;
      // 目标状态一律写归属标签（tab 是唯一事实源，不再写 store）
      tab.goalText = null;
      tab.goalStatus = null;
      tab.goalArmed = false;
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
