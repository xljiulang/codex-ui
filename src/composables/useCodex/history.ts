import { invoke } from "@tauri-apps/api/core";
import { markRaw } from "vue";
import {
  applyHistoryDetailPage,
  composeHistoryItems,
  createHistoryTurnStates,
  emptyHistoryState,
  findNextOlderDetailTurnId,
  type HistoryDetailRecord,
  type HistoryTurnState,
  type SessionHistoryState,
} from "../../lib/historyWindow";
import type { ThreadItem, Turn } from "../../lib/types";
import { enrichUserMessage } from "../../lib/userMessage";
import { isActiveItem } from "./items";
import { store } from "./store";
import type { SessionTab } from "./types";

const SUMMARY_PAGE_SIZE = 50;
const DETAIL_PAGE_SIZE = 100;
const INITIAL_DETAIL_TURNS = 2;
const MAX_SUMMARY_PAGES = 200;

interface TurnsPage {
  data?: Turn[];
  nextCursor?: string | null;
}

interface ItemsPage {
  data?: HistoryDetailRecord[];
  nextCursor?: string | null;
}

function setHistory(tab: SessionTab, history: SessionHistoryState): void {
  tab.history = markRaw(history);
}

function syncHistoryItems(tab: SessionTab): void {
  const threadId = tab.threadId;
  const history = tab.history;
  if (!threadId || !history) return;
  const composed = composeHistoryItems(history.turns);
  const existing = store.itemsByThread[threadId] ?? [];
  const liveById = new Map(existing.map((item) => [item.id, item]));
  const composedIds = new Set(composed.map((item) => item.id));
  const items = composed.map((item) => {
    const live = liveById.get(item.id);
    return live && isActiveItem(live) ? live : item;
  });
  for (const item of existing) {
    if (!composedIds.has(item.id)) items.push(item);
  }
  store.itemsByThread[threadId] = items;
  store.activeWorkByThread[threadId] = items.filter(isActiveItem).length;
  store.itemsRev++;
}

function replaceTurn(
  history: SessionHistoryState,
  turnId: string,
  update: (turn: HistoryTurnState) => HistoryTurnState,
): SessionHistoryState {
  return {
    ...history,
    turns: history.turns.map((turn) =>
      turn.id === turnId ? update(turn) : turn,
    ),
  };
}

function itemWithTurnTime(
  item: ThreadItem,
  turn: HistoryTurnState,
): ThreadItem {
  const next =
    typeof item.startedAtMs === "number" || turn.startedAtMs === null
      ? item
      : { ...item, startedAtMs: turn.startedAtMs };
  return enrichUserMessage(next);
}

async function fetchDetailPage(
  threadId: string,
  turnId: string,
  cursor: string | null,
): Promise<ItemsPage> {
  return invoke<ItemsPage>("codex_rpc", {
    method: "thread/items/list",
    params: {
      threadId,
      turnId,
      cursor,
      limit: DETAIL_PAGE_SIZE,
      sortDirection: "desc",
      itemView: "full",
    },
  });
}

async function loadSummaryTurns(threadId: string): Promise<HistoryTurnState[]> {
  const turns: Turn[] = [];
  let cursor: string | null = null;
  for (let page = 0; page < MAX_SUMMARY_PAGES; page++) {
    const res: TurnsPage = await invoke<TurnsPage>("codex_rpc", {
      method: "thread/turns/list",
      params: {
        threadId,
        cursor,
        limit: SUMMARY_PAGE_SIZE,
        sortDirection: "desc",
        itemsView: "summary",
      },
    });
    const rows = res.data ?? [];
    turns.push(...rows);
    cursor = res.nextCursor ?? null;
    if (!cursor || rows.length === 0) break;
  }
  return createHistoryTurnStates(turns);
}

async function loadInitialDetail(
  turns: HistoryTurnState[],
  threadId: string,
): Promise<HistoryTurnState[]> {
  const targets = turns.slice(-INITIAL_DETAIL_TURNS);
  const results = await Promise.allSettled(
    targets.map(async (turn) => {
      const page = await fetchDetailPage(threadId, turn.id, null);
      const items = (page.data ?? [])
        .filter((record) => record.turnId === turn.id)
        .map((record) => itemWithTurnTime(record.item, turn));
      return applyHistoryDetailPage(turn, items, page.nextCursor ?? null);
    }),
  );
  const byId = new Map<string, HistoryTurnState>();
  results.forEach((result, index) => {
    const base = targets[index];
    byId.set(
      base.id,
      result.status === "fulfilled"
        ? result.value
        : {
            ...base,
            detailLoading: false,
            detailError:
              result.reason instanceof Error
                ? result.reason.message
                : String(result.reason),
          },
    );
  });
  return turns.map((turn) => byId.get(turn.id) ?? turn);
}

/**
 * 建立完整摘要骨架，并只加载最新两个回合各一页明细。
 * 返回 false 表示 summary 能力不可用，由调用方走旧版全量回退。
 */
export async function loadThreadHistory(
  tab: SessionTab,
  threadId: string,
): Promise<boolean> {
  setHistory(tab, {
    ...emptyHistoryState(),
    loading: true,
  });
  try {
    const skeleton = await loadSummaryTurns(threadId);
    const turns = await loadInitialDetail(skeleton, threadId);
    setHistory(tab, {
      turns,
      loaded: true,
      loading: false,
      loadingOlder: false,
      error: null,
    });
    // 新打开标签的窗口以服务端摘要/明细为准；清掉测试或旧重载残留的同线程缓存。
    store.itemsByThread[threadId] = [];
    syncHistoryItems(tab);
    return true;
  } catch (e) {
    console.warn("[codex-ui] 历史窗口加载失败，回退旧版加载:", e);
    setHistory(tab, {
      ...emptyHistoryState(),
      error: e instanceof Error ? e.message : String(e),
    });
    return false;
  }
}

/**
 * 为指定回合加载下一更早明细页。未加载回合从游标 null 开始；
 * 已加载回合继续其游标。同一时刻只允许一个历史明细请求。
 */
export async function loadHistoryDetailPage(
  tab: SessionTab,
  threadId: string,
  turnId: string,
): Promise<boolean> {
  const current = tab.history;
  if (!current?.loaded || current.loadingOlder) return false;
  const turn = current.turns.find((item) => item.id === turnId);
  if (!turn || (turn.detailLoaded && !turn.detailHasMore)) return false;
  const loading = replaceTurn(
    { ...current, loadingOlder: true },
    turnId,
    (item) => ({ ...item, detailLoading: true, detailError: null }),
  );
  setHistory(tab, loading);
  try {
    const page = await fetchDetailPage(threadId, turnId, turn.detailCursor);
    const items = (page.data ?? [])
      .filter((record) => record.turnId === turnId)
      .map((record) => itemWithTurnTime(record.item, turn));
    const done = replaceTurn(
      { ...loading, loadingOlder: false },
      turnId,
      (item) => applyHistoryDetailPage(item, items, page.nextCursor ?? null),
    );
    setHistory(tab, done);
    syncHistoryItems(tab);
    return true;
  } catch (e) {
    const failed = replaceTurn(
      { ...loading, loadingOlder: false },
      turnId,
      (item) => ({
        ...item,
        detailLoading: false,
        detailError: e instanceof Error ? e.message : String(e),
      }),
    );
    setHistory(tab, failed);
    return false;
  }
}

/** 自动补载连续明细窗口的下一更早页；没有更早回合时返回 false。 */
export async function loadOlderHistoryPage(
  tab: SessionTab,
  threadId: string,
): Promise<boolean> {
  const turnId = tab.history
    ? findNextOlderDetailTurnId(tab.history.turns)
    : null;
  if (!turnId) return false;
  const turn = tab.history?.turns.find((item) => item.id === turnId);
  if (turn?.detailLoaded && !turn.detailHasMore) return false;
  return loadHistoryDetailPage(tab, threadId, turnId);
}

/** 回合导航点击后按其用户消息找到回合，并只补载该回合下一更早明细页。 */
export async function loadHistoryDetailForUserMessage(
  tab: SessionTab,
  threadId: string,
  userMessageId: string,
): Promise<boolean> {
  const turn = tab.history?.turns.find((item) =>
    item.summaryItems.some(
      (message) =>
        message.type === "userMessage" && message.id === userMessageId,
    ),
  );
  if (!turn) return false;
  return loadHistoryDetailPage(tab, threadId, turn.id);
}

function upsertHistoryItem(
  items: readonly ThreadItem[],
  item: ThreadItem,
): ThreadItem[] {
  const index = items.findIndex((existing) => existing.id === item.id);
  if (index < 0) return [...items, item];
  const next = [...items];
  next[index] = item;
  return next;
}

/**
 * 实时事件同步写入摘要/明细源。渲染仍由 upsertItem 更新 store，
 * 这里只保证之后补载其他历史页、重新合成时不会覆盖流式内容。
 */
export function mirrorHistoryItem(
  tab: SessionTab,
  turnId: string,
  item: ThreadItem,
): void {
  const history = tab.history;
  if (!history?.loaded) return;
  let found = false;
  const turns = history.turns.map((turn) => {
    const summaryIndex = turn.summaryItems.findIndex(
      (message) => message.id === item.id,
    );
    if (turn.id !== turnId && summaryIndex < 0) return turn;
    found = true;
    const nextSummary =
      summaryIndex >= 0
        ? upsertHistoryItem(turn.summaryItems, item)
        : turn.summaryItems;
    if (turn.id !== turnId) return { ...turn, summaryItems: nextSummary };
    return {
      ...turn,
      summaryItems: nextSummary,
      detailItems: turn.detailLoaded
        ? upsertHistoryItem(turn.detailItems, item)
        : turn.detailItems,
    };
  });
  if (!found) {
    const startedAtMs =
      typeof item.startedAtMs === "number" ? item.startedAtMs : null;
    turns.push({
      id: turnId,
      startedAtMs,
      summaryItems: item.type === "userMessage" ? [item] : [],
      detailItems: [item],
      detailCursor: null,
      detailHasMore: false,
      detailLoaded: true,
      detailLoading: false,
      detailError: null,
    });
  }
  setHistory(tab, { ...history, turns });
}
