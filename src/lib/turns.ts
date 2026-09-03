import { FILE_MENTION_HEADING, MY_REQUEST_MARKER } from "./mention";
import { isUserInput, type ThreadItem } from "./types";

export type TurnRow =
  | { key: string; kind: "sep"; date: string }
  | { key: string; kind: "msg"; item: ThreadItem };

export interface Turn {
  key: string;
  rows: TurnRow[];
}

export type DayFormatter = (ts: number) => string;

/**
 * 回合定位容差（px）：用户消息顶边刚越过/正好位于视口顶边时仍视为当前回合，
 * 吸收浮点误差与像素取整。
 */
const TURN_NAV_TOLERANCE_PX = 2;

/**
 * 返回最后一个“顶边不高于视口顶部（含容差）”的用户消息锚点下标；
 * 锚点坐标应按内容顺序递增，找不到（视口在首个锚点上方或列表为空）返回 -1。
 */
export function findCurrentTurnIndex(
  anchorTops: readonly number[],
  scrollTop: number,
): number {
  const limit = scrollTop + TURN_NAV_TOLERANCE_PX;
  let lo = -1;
  let hi = anchorTops.length;
  while (lo + 1 < hi) {
    const mid = (lo + hi) >> 1;
    if (anchorTops[mid] <= limit) lo = mid;
    else hi = mid;
  }
  return lo;
}

/** 把 Markdown 源文本转为卡片预览用的纯文本（不执行渲染，仅剥离常见语法） */
export function markdownToPlainText(text: string): string {
  return text
    .replace(/```[\s\S]*?(?:```|$)/g, " ")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/^>\s?/gm, "")
    .replace(/^\s*(?:[-*+]|\d+[.)])\s+/gm, "")
    .replace(/[*_~]{1,3}/g, "")
    .replace(/<br\s*\/?>/gi, " ")
    .replace(/[ \t]+/g, " ")
    .replace(/\s*\n\s*/g, " ")
    .trim();
}

/** 用户消息的导航预览：提取正文并转纯文本；无正文时回退附件/引用名称 */
export function turnPreviewText(item: ThreadItem): string {
  if (item.type !== "userMessage") return "";
  const content = Array.isArray(item.content) ? item.content : [];
  const parts: string[] = [];
  for (const c of content) {
    if (!isUserInput(c)) continue;
    if (c.type === "text") {
      const marker = `\n${MY_REQUEST_MARKER}\n`;
      const idx = c.text.indexOf(marker);
      const body =
        idx >= 0 && c.text.includes(FILE_MENTION_HEADING)
          ? c.text.slice(idx + marker.length)
          : c.text;
      const plain = markdownToPlainText(body);
      if (plain) parts.push(plain);
    } else if (c.type === "localImage") {
      parts.push("[图片]");
    } else if (c.type === "mention") {
      parts.push(`@${c.name}`);
    } else if (c.type === "skill") {
      parts.push(`$${c.name}`);
    }
  }
  return parts.join(" ");
}

/**
 * 把消息序列按“回合”分组：userMessage 起始新回合，其后所有条目归入当前回合；
 * 首个用户消息之前的条目（历史续接）归入单一伪回合。
 * 日期分隔线随条目插入其所属回合（无时间戳的条目不产生分隔线）。
 */
export function createTurnsBuilder(formatDay: DayFormatter) {
  // 行对象按 key 缓存复用，未变化的消息不再重复分配（降低大对话下的 GC 抖动）
  const rowCache = new Map<string, TurnRow>();

  return {
    build(items: readonly ThreadItem[]): Turn[] {
      const turns: Turn[] = [];
      let lastDay = "";
      // 每轮 build 内计数：同一日期在同一次构建中出现多次时 key 仍唯一；
      // 跨 build 计数重置，保证同一位置的日期分隔线复用同一行对象与 key，
      // 避免流式重算期间缓存条目随构建次数无限增长。
      let sepCount = 0;

      // 前置伪回合（首个用户消息之前的条目）：先收集，确定 key 后再入列
      let preKey: string | null = null;
      let preRows: TurnRow[] = [];

      // 当前用户回合
      let turnKey: string | null = null;
      let turnRows: TurnRow[] = [];

      const rowFor = (item: ThreadItem): TurnRow => {
        let row = rowCache.get(item.id);
        if (!row || row.kind !== "msg" || row.item !== item) {
          row = { key: item.id, kind: "msg", item };
          rowCache.set(item.id, row);
        }
        return row;
      };

      const sepRow = (day: string): TurnRow => {
        const key = `sep-${day}-${sepCount}`;
        let row = rowCache.get(key);
        if (!row) {
          row = { key, kind: "sep", date: day };
          rowCache.set(key, row);
        }
        sepCount++;
        return row;
      };

      const closeTurn = () => {
        if (turnKey !== null && turnRows.length > 0) {
          turns.push({ key: turnKey, rows: turnRows });
        }
        turnKey = null;
        turnRows = [];
      };

      const flushPre = () => {
        if (preRows.length > 0) {
          turns.push({ key: preKey ?? `pre-${preRows[0].key}`, rows: preRows });
        }
        preKey = null;
        preRows = [];
      };

      for (const item of items) {
        // 先确定该条目所属回合
        if (item.type === "userMessage") {
          closeTurn();
          flushPre();
          turnKey = `turn-${item.id}`;
          turnRows = [];
        } else if (turnKey === null) {
          if (preKey === null) preKey = `pre-${item.id}`;
        }

        // 日期分隔线随条目插入其所属回合
        const ts = item.startedAtMs as number | undefined;
        if (typeof ts === "number") {
          const day = formatDay(ts);
          if (day !== lastDay) {
            const sep = sepRow(day);
            if (turnKey === null) preRows.push(sep);
            else turnRows.push(sep);
            lastDay = day;
          }
        }

        const row = rowFor(item);
        if (turnKey === null) preRows.push(row);
        else turnRows.push(row);
      }
      closeTurn();
      flushPre();
      return turns;
    },

    clear(): void {
      rowCache.clear();
    },
  };
}

/** 一次性构建（每次新建缓存），用于测试与简单场景 */
export function buildTurns(
  items: readonly ThreadItem[],
  formatDay: DayFormatter,
): Turn[] {
  return createTurnsBuilder(formatDay).build(items);
}
