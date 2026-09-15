// useCodex 拆分模块：事件桥（原 useCodex.ts 的一部分，纯移动，行为不变）
import { watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, ProgressBarStatus } from "@tauri-apps/api/window";
import {
  friendlyServerError,
  friendlyServerMessage,
} from "../../lib/serverMessages";
import {
  type CodexMessageEvent,
  type ErrorItem,
  isThreadItemType,
  type AgentMessageItem,
  type CommandExecutionItem,
  type McpToolCallItem,
  type ReasoningItem,
  type ScheduledTask,
  type ServerStatus,
  type ThreadItem,
} from "../../lib/types";
import { sessionLog } from "../../lib/sessionLog";
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
import {
  interactionKindForMethod,
  notifySessionError,
  notifySessionInteraction,
} from "./sessionNotify";
import { clearGoal, startTurn, startTurnForTab } from "./turnControl";
import { handleDynamicToolCall } from "./dynamicToolCall";
import { tabs } from "../useTabs";
import { isTabWorking } from "../../lib/tabs";
import {
  goalStatusToast,
  isGoalStatus,
  isGoalTerminalStatus,
  type PlanStep,
  type SessionTab,
} from "./types";


let unlisteners: UnlistenFn[] = [];
let wired = false;


/** 任一标签（会话回合/目标续跑、终端命令执行）在工作时，任务栏显示不确定进度条；
 * 全部结束后隐藏（非 Tauri 环境静默忽略）。与标签栏呼吸灯同源（isTabWorking）。 */
async function updateTaskbarProgress() {
  try {
    const win = getCurrentWindow();
    await win.setProgressBar({
      status: tabs.some((t) => isTabWorking(t))
        ? ProgressBarStatus.Indeterminate
        : ProgressBarStatus.None,
    });
  } catch {
    // 非 Tauri 环境（如浏览器预览）忽略
  }
}


watch(
  () => tabs.some((t) => isTabWorking(t)),
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


/**
 * 会话内 error 条目（`item.type === "error"`，服务端错误消息）：窗口没有前台焦点时
 * 额外发一条 Windows 通知，让用户切走后也能看到该错误（应用内错误卡片照旧渲染）。
 */
function notifyErrorItem(
  item: ThreadItem,
  threadId: string,
  turnId?: string,
): void {
  if (!isThreadItemType<ErrorItem>(item, "error")) {
    return;
  }
  const message = String(item.message ?? "").trim();
  if (!message) return;
  notifySessionError({ message, threadId, turnId });
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
    await listen("taskbar-progress-refresh", () => {
      // 窗口从托盘恢复后重新应用任务栏进度：工作态未变时 watch 不会主动触发，
      // 而 Windows 任务栏按钮重建会丢失进度条 overlay，故在此按当前状态重算一次。
      void updateTaskbarProgress();
    }),
  );

  unlisteners.push(
    await listen("interaction:request", async (e) => {
      const p = e.payload as {
        requestId: number | string;
        method: string;
        params: Record<string, unknown>;
      };
      // codexui 动态工具调用（item/tool/call）：直接判定并应答，不进入交互气泡
      if (p.method === "item/tool/call") {
        void handleDynamicToolCall(p);
        return;
      }
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
      // 窗口没有前台焦点时发系统通知（应用内气泡行为不变；无会话归属时不发）
      notifySessionInteraction({
        kind: interactionKindForMethod(p.method),
        threadId,
      });
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
    await listen("thread/settings/updated", (e) => {
      const p = e.payload as {
        threadId?: string;
        threadSettings?: { collaborationMode?: { mode?: string } };
      };
      const tid = p.threadId;
      if (!tid || isBackgroundThread(tid)) return;
      const tab = findSessionTabByThread(tid);
      if (!tab) return;
      // 服务端「下一回合」实际生效的协作模式（权威事件源），与本地 collaborationMode 对账；
      // 缺失/未知取值静默忽略，不 toast 不抛错（与现有 turn 事件防御一致）
      const serverMode = p.threadSettings?.collaborationMode?.mode;
      if (serverMode !== "plan" && serverMode !== "default") return;
      if (tab.collaborationMode !== serverMode) {
        const prev = tab.collaborationMode;
        tab.collaborationMode = serverMode;
        sessionLog(
          "warn",
          tid,
          "task-mode-reconcile",
          `prev=${prev} -> server=${serverMode}`,
        );
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
      tab.plan = null; // 新回合开始：重置 Updated Plan 任务清单
      // currentTurnId 保留 turn/start 响应的服务端回合 id（turn_interrupt 需要）；
      // 事件 id 仅在响应缺失时兜底。
      if (!tab.currentTurnId && p.turn?.id) tab.currentTurnId = p.turn.id;
    }),
    await listen("turn/completed", async (e) => {
      const p = e.payload as {
        threadId?: string;
        turn?: {
          id?: string;
          status?: string;
          error?: { message?: string; codexErrorInfo?: unknown } | null;
        };
      };
      if (isBackgroundThread(p.threadId)) return; // 后台临时线程完成不影响主对话
      // 回合失败：错误正文写在 turn.error（`error` 通知可能早于本事件到达，
      // 也可能是唯一来源）；仅有 threadId 时才发系统通知
      const turnError = p.turn?.status === "failed" ? p.turn.error : null;
      const turnErrorMessage = String(turnError?.message ?? "").trim();
      if (turnErrorMessage && p.threadId) {
        notifySessionError({
          message: turnErrorMessage,
          codexErrorInfo: turnError?.codexErrorInfo,
          threadId: p.threadId,
          turnId: p.turn?.id,
        });
      }
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
        tab.collaborationMode === "plan" &&
        !interrupted &&
        tab.followupQueue.length === 0 &&
        p.turn?.id &&
        tid &&
        tab.planPrompt?.turnId !== p.turn.id
      ) {
        const threadItems = store.itemsByThread[tid] ?? [];
        // 只认可“当前回合”产出的 plan：回合以 userMessage 起始，plan 必须位于
        // 最后一条 userMessage 之后；找不到 userMessage（历史/测试 fixture）时
        // 回退整线程扫描，避免把上一个回合的旧计划误当本轮产出重复弹窗。
        let scanStart = 0;
        for (let i = threadItems.length - 1; i >= 0; i--) {
          if (threadItems[i]?.type === "userMessage") {
            scanStart = i + 1;
            break;
          }
        }
        let planText = "";
        for (let i = threadItems.length - 1; i >= scanStart; i--) {
          const it = threadItems[i];
          if (it?.type === "plan" && typeof it.text === "string" && it.text.trim()) {
            planText = it.text;
            break;
          }
        }
        if (planText) {
          tab.planPrompt = { threadId: tid, turnId: p.turn.id, planText };
          // 窗口没有前台焦点时发系统通知（计划确认气泡行为不变）
          notifySessionInteraction({ kind: "plan", threadId: tid });
        }
      }
      await refreshThreads();
      // 处理“加入队列”的跟进消息
      if (tab && tab.followupQueue.length) {
        const next = tab.followupQueue.shift()!;
        if (activeSessionTab()?.id === tab.id) {
          // 活动标签：走 live 字段路径（发送状态落到当前会话）
          await startTurn(next.text, next.attachments);
        } else if (tab.threadId) {
          // 后台标签：状态写入标签记录，不触碰活动标签
          await startTurnForTab(tab, next.text, next.attachments);
        }
      }
    }),
  );

  unlisteners.push(
    await listen("item/started", (e) => {
      const p = e.payload as {
        item: ThreadItem;
        threadId: string;
        turnId?: string;
        startedAtMs?: number;
      };
      if (isBackgroundThread(p.threadId)) return;
      notifyErrorItem(p.item, p.threadId, p.turnId);
      upsertItem(p.threadId, {
        ...p.item,
        startedAtMs: p.startedAtMs ?? Date.now(),
      });
    }),
    await listen("item/completed", (e) => {
      const p = e.payload as {
        item: ThreadItem;
        threadId: string;
        turnId?: string;
        completedAtMs?: number;
      };
      if (isBackgroundThread(p.threadId)) return;
      notifyErrorItem(p.item, p.threadId, p.turnId);
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
    await listen("turn/plan/updated", (e) => {
      const p = e.payload as {
        threadId?: string;
        turnId?: string | number;
        turn?: { id?: string };
        explanation?: unknown;
        plan?: unknown;
      };
      const threadId = typeof p.threadId === "string" ? p.threadId : undefined;
      if (isBackgroundThread(threadId)) return;
      const tab = threadId
        ? resolveTurnTab({ threadId })
        : typeof p.turnId === "string" || typeof p.turnId === "number"
          ? resolveTurnTab({ turn: { id: String(p.turnId) } })
          : p.turn?.id
            ? resolveTurnTab({ turn: { id: p.turn.id } })
            : null;
      if (!tab) return;
      // 载荷防御性归一化：非法/缺失 status 一律按 pending 处理
      const rawSteps = Array.isArray(p.plan) ? p.plan : [];
      const steps = rawSteps.flatMap((s) => {
        if (!s || typeof s !== "object") return [];
        const o = s as Record<string, unknown>;
        if (typeof o.step !== "string" || !o.step) return [];
        return {
          step: o.step,
          status:
            o.status === "inProgress" || o.status === "completed"
              ? o.status
              : "pending",
        };
      }) as PlanStep[];
      tab.plan = {
        ...(typeof p.explanation === "string" && p.explanation
          ? { explanation: p.explanation }
          : {}),
        steps,
      };
    }),
  );

  unlisteners.push(
    await listen("item/reasoning/summaryTextDelta", (e) => {
      const p = e.payload as {
        threadId?: string;
        itemId?: string;
        delta?: string;
        summaryIndex?: number;
      };
      if (!p.threadId || !p.itemId) return;
      const threadId = p.threadId;
      const itemId = p.itemId;
      if (isBackgroundThread(threadId)) return;
      const item = getOrCreateItem(threadId, itemId, () => ({
        id: itemId,
        type: "reasoning",
        summary: [],
        streaming: true,
      }));
      if (!isThreadItemType<ReasoningItem>(item, "reasoning")) return;
      if (typeof item.startedAtMs !== "number") item.startedAtMs = Date.now();
      if (!item.streaming) {
        item.streaming = true;
        bumpActive(threadId, 1);
      }
      const summary = item.summary ?? [];
      const idx =
        typeof p.summaryIndex === "number"
          ? p.summaryIndex
          : summary.length - 1;
      if (idx >= 0 && idx < summary.length) {
        summary[idx] = (summary[idx] ?? "") + String(p.delta ?? "");
      } else if (idx < 0 || idx === summary.length) {
        summary.push(String(p.delta ?? ""));
      } else {
        while (summary.length <= idx) summary.push("");
        summary[idx] = String(p.delta ?? "");
      }
      item.summary = summary;
      store.itemsRev++;
    }),
  );

  unlisteners.push(
    await listen("item/reasoning/summaryPartAdded", (e) => {
      const p = e.payload as {
        threadId?: string;
        itemId?: string;
        summaryIndex?: number;
      };
      if (!p.threadId || !p.itemId) return;
      const threadId = p.threadId;
      const itemId = p.itemId;
      if (isBackgroundThread(threadId)) return;
      const item = getOrCreateItem(threadId, itemId, () => ({
        id: itemId,
        type: "reasoning",
        summary: [],
        streaming: true,
      }));
      if (!isThreadItemType<ReasoningItem>(item, "reasoning")) return;
      const summary = item.summary ?? [];
      const idx =
        typeof p.summaryIndex === "number" ? p.summaryIndex : summary.length;
      while (summary.length <= idx) summary.push("");
      item.summary = summary;
      store.itemsRev++;
    }),
  );

  unlisteners.push(
    await listen("item/mcpToolCall/progress", (e) => {
      const p = e.payload as {
        threadId?: string;
        itemId?: string;
        message?: unknown;
        text?: unknown;
        progress?: unknown;
        percent?: unknown;
      };
      if (!p.threadId || !p.itemId) return;
      const threadId = p.threadId;
      const itemId = p.itemId;
      if (isBackgroundThread(threadId)) return;
      const item = getOrCreateItem(threadId, itemId, () => ({
        id: itemId,
        type: "mcpToolCall",
        server: "",
        tool: "",
        status: "in_progress",
      }));
      if (!isThreadItemType<McpToolCallItem>(item, "mcpToolCall")) return;
      const progressText = String(p.message ?? p.text ?? p.progress ?? "");
      if (progressText) item.progressText = progressText;
      const percent = typeof p.percent === "number" ? p.percent : NaN;
      if (!Number.isNaN(percent) && percent >= 0 && percent <= 100) {
        item.progressPercent = percent;
      }
      store.itemsRev++;
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
          total?: {
            totalTokens?: number;
            inputTokens?: number;
            outputTokens?: number;
            cachedInputTokens?: number;
            cacheWriteInputTokens?: number;
            reasoningOutputTokens?: number;
          };
          last?: { totalTokens?: number };
          modelContextWindow?: number | null;
        };
      };
      // 会话累计的输入/输出/细分 token（用于输入区圆环菜单展示）；缺省不写，避免污染为 undefined
      const t = p.tokenUsage?.total;
      const totalInput = t?.inputTokens;
      const totalOutput = t?.outputTokens;
      const totalAll = t?.totalTokens;
      const cachedInput = t?.cachedInputTokens;
      const cacheWriteInput = t?.cacheWriteInputTokens;
      const reasoningOutput = t?.reasoningOutputTokens;
      const usage = {
        // 当前上下文占用取 last（最近一次请求），total 为会话累计（会超过窗口）
        contextUsed:
          p.tokenUsage?.last?.totalTokens ?? p.tokenUsage?.total?.totalTokens ?? 0,
        window: p.tokenUsage?.modelContextWindow ?? null,
        ...(typeof totalInput === "number" ? { input: totalInput } : {}),
        ...(typeof totalOutput === "number" ? { output: totalOutput } : {}),
        ...(typeof totalAll === "number" ? { totalTokens: totalAll } : {}),
        ...(typeof cachedInput === "number" ? { cachedInput } : {}),
        ...(typeof cacheWriteInput === "number" ? { cacheWriteInput } : {}),
        ...(typeof reasoningOutput === "number" ? { reasoningOutput } : {}),
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
    await listen("codex/message", (e) => {
      const p = e.payload as CodexMessageEvent;
      if (!p?.message) return;
      if (p.level === "error") {
        setToast(
          friendlyServerError({
            error: { message: p.message, codexErrorInfo: p.codexErrorInfo },
          }),
        );
        // 窗口没有前台焦点时同时投 Windows 通知（应用内 toast 行为不变）
        notifySessionError({
          message: p.message,
          codexErrorInfo: p.codexErrorInfo,
          threadId: p.threadId,
          turnId: p.turnId,
        });
        return;
      }
      // 非 error 只会在 DEBUG 构建由后端发送；Release 下 warning 仅落盘
      setToast(friendlyServerMessage(p.message));
    }),
  );

  // 定时任务快照：后端任务/执行记录变更时全量推送任务列表
  unlisteners.push(
    await listen("scheduled-tasks/event", (e) => {
      const p = e.payload as { tasks?: ScheduledTask[]; changedTaskId?: string };
      if (Array.isArray(p.tasks)) store.scheduledTasks = p.tasks;
      // seq 递增：同一任务连续两次事件（如 started → completed）也能被 watch 感知
      store.scheduledTaskChange = {
        seq: store.scheduledTaskChange.seq + 1,
        taskId: p.changedTaskId ?? "",
      };
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
