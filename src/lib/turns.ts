import type { ThreadItem } from "./types";

export type TurnRow =
  | { key: string; kind: "sep"; date: string }
  | { key: string; kind: "msg"; item: ThreadItem };

export interface Turn {
  key: string;
  rows: TurnRow[];
}

export type DayFormatter = (ts: number) => string;

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
