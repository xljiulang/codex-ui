import type { ThreadItem, Turn } from "./types";
import {
  capTextTail,
  OUTPUT_TEXT_BYTE_CAP,
  OUTPUT_TEXT_LINE_CAP,
} from "./textWindow";

/** 单回合可用的历史明细页（响应可能跨回合，落库前按 turnId 分组）。 */
export interface HistoryDetailRecord {
  turnId: string;
  item: ThreadItem;
}

/** 单回合的轻量摘要骨架与已加载明细窗口。 */
export interface HistoryTurnState {
  id: string;
  startedAtMs: number | null;
  summaryItems: ThreadItem[];
  detailItems: ThreadItem[];
  detailCursor: string | null;
  detailHasMore: boolean;
  detailLoaded: boolean;
  detailLoading: boolean;
  detailError: string | null;
}

/** 会话标签持有的一页历史窗口状态；数组本体走 markRaw，避免逐条深度代理。 */
export interface SessionHistoryState {
  turns: HistoryTurnState[];
  loaded: boolean;
  loading: boolean;
  loadingOlder: boolean;
  error: string | null;
}

/** 历史命令输出仍按 UI 现有上限保留末尾 5000 行。 */
export const HISTORY_OUTPUT_LINE_CAP = OUTPUT_TEXT_LINE_CAP;

export function emptyHistoryState(): SessionHistoryState {
  return {
    turns: [],
    loaded: false,
    loading: false,
    loadingOlder: false,
    error: null,
  };
}

function withTurnTime(turn: Turn, item: ThreadItem): ThreadItem {
  const startedAtMs =
    typeof item.startedAtMs === "number"
      ? item.startedAtMs
      : typeof turn.startedAt === "number"
        ? turn.startedAt * 1000
        : undefined;
  return startedAtMs === undefined || item.startedAtMs === startedAtMs
    ? item
    : { ...item, startedAtMs };
}

/**
 * summary 页按 newest→oldest 返回；这里转成时间正序，并挂上回合开始时间，
 * 供聊天日期分隔线、导航标题和后续明细合成共同使用。
 */
export function createHistoryTurnStates(
  turnsDesc: readonly Turn[],
): HistoryTurnState[] {
  return [...turnsDesc].reverse().map((turn) => ({
    id: turn.id,
    startedAtMs:
      typeof turn.startedAt === "number" ? turn.startedAt * 1000 : null,
    summaryItems: (turn.items ?? []).map((item) => withTurnTime(turn, item)),
    detailItems: [],
    detailCursor: null,
    detailHasMore: true,
    detailLoaded: false,
    detailLoading: false,
    detailError: null,
  }));
}

/** 历史命令只保留 UI 能展示的末尾窗口；未超限时返回原对象。 */
export function capHistoryCommandOutput(item: ThreadItem): ThreadItem {
  if (item.type !== "commandExecution") return item;
  const output = item.aggregatedOutput;
  if (typeof output !== "string") return item;
  const capped = capTextTail(output, {
    maxLines: HISTORY_OUTPUT_LINE_CAP,
    maxBytes: OUTPUT_TEXT_BYTE_CAP,
  });
  if (capped.truncation === "none") return item;
  return {
    ...item,
    aggregatedOutput: capped.text,
    _historyOutputTruncated: true,
    _historyOutputTruncation: capped.truncation,
  };
}

function uniqueById(items: readonly ThreadItem[]): ThreadItem[] {
  const seen = new Set<string>();
  const out: ThreadItem[] = [];
  for (const item of items) {
    if (seen.has(item.id)) continue;
    seen.add(item.id);
    out.push(item);
  }
  return out;
}

/**
 * 明细页按 newest→oldest 返回。每页先反转，再整体插到已加载明细之前；
 * 重复 id 保留先出现项，防止分页边界重复创建组件。
 */
export function applyHistoryDetailPage(
  turn: HistoryTurnState,
  pageNewestFirst: readonly ThreadItem[],
  nextCursor: string | null,
): HistoryTurnState {
  const older = pageNewestFirst.map(capHistoryCommandOutput).reverse();
  return {
    ...turn,
    detailItems: uniqueById([...older, ...turn.detailItems]),
    detailCursor: nextCursor ?? null,
    detailHasMore: !!nextCursor,
    detailLoaded: true,
    detailLoading: false,
    detailError: null,
  };
}

/**
 * 未加载回合只显示用户消息与最终答复；有明细时摘要只补齐明细缺失的首尾，
 * 中间完整保留思考、工具和命令项。
 */
export function composeHistoryTurnItems(turn: HistoryTurnState): ThreadItem[] {
  if (!turn.detailLoaded) return uniqueById(turn.summaryItems);
  const detailIds = new Set(turn.detailItems.map((item) => item.id));
  const leading = turn.summaryItems.filter(
    (item) => item.type === "userMessage" && !detailIds.has(item.id),
  );
  const trailing = turn.summaryItems.filter(
    (item) => item.type !== "userMessage" && !detailIds.has(item.id),
  );
  return uniqueById([...leading, ...turn.detailItems, ...trailing]);
}

/** 将全部回合合成为现有 ChatView 可直接消费的连续消息列表。 */
export function composeHistoryItems(
  turns: readonly HistoryTurnState[],
): ThreadItem[] {
  return turns.flatMap(composeHistoryTurnItems);
}

/**
 * 自动向前加载的连续边界：从最新回合回溯“已加载明细”的起点；
 * 该回合还有更早页时继续补它，否则进入紧邻的上一回合。
 */
export function findNextOlderDetailTurnId(
  turns: readonly HistoryTurnState[],
): string | null {
  let boundary = turns.length - 1;
  while (boundary >= 0 && turns[boundary].detailLoaded) {
    if (turns[boundary].detailHasMore) return turns[boundary].id;
    boundary--;
  }
  return turns[boundary]?.id ?? null;
}
