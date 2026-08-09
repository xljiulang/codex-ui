export interface VirtualRowLike {
  kind: string;
  item?: { type?: string } | null;
}

const TOOL_TYPES = [
  "commandExecution",
  "mcpToolCall",
  "dynamicToolCall",
  "collabAgentToolCall",
  "webSearch",
  "fileChange",
  "todoList",
];

/** 未测量前的默认行高（按消息类型估计，测量后会被实测值覆盖） */
export function estimateRowHeight(row: VirtualRowLike): number {
  if (row.kind === "sep") return 34;
  const type = row.item?.type ?? "";
  if (type === "userMessage") return 60;
  if (type === "agentMessage" || type === "plan") return 180;
  if (type === "reasoning") return 80;
  if (type === "imageView") return 340;
  if (TOOL_TYPES.includes(type)) return 110;
  return 60;
}

/** 行高前缀和：prefix[i] = 第 0..i 行高度之和（即第 i 行的底部偏移） */
export function buildPrefixHeights<T extends VirtualRowLike>(
  rows: T[],
  heightFn: (row: T, index: number) => number,
): number[] {
  const prefix = new Array<number>(rows.length);
  let sum = 0;
  for (let i = 0; i < rows.length; i++) {
    sum += heightFn(rows[i], i);
    prefix[i] = sum;
  }
  return prefix;
}

/**
 * 计算可视行区间（含上下 overscan）。
 * 尺寸不可测（viewport<=0）或总高为 0 时返回全量，保证不丢内容。
 */
export function computeVisibleRange(
  prefix: number[],
  scrollTop: number,
  viewport: number,
  overscan = 5,
): { start: number; end: number } {
  const n = prefix.length;
  if (n === 0) return { start: 0, end: 0 };
  const total = prefix[n - 1];
  if (viewport <= 0 || total <= 0) return { start: 0, end: n - 1 };

  const top = Math.max(0, scrollTop);
  const bottom = top + viewport;

  // start：第一个底部 > top 的行
  let lo = 0;
  let hi = n - 1;
  let start = 0;
  if (top >= total) {
    start = n - 1;
  } else {
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (prefix[mid] > top) {
        start = mid;
        hi = mid - 1;
      } else {
        lo = mid + 1;
      }
    }
  }

  // end：最后一个顶部 < bottom 的行（行 i 顶部 = i===0 ? 0 : prefix[i-1]）
  lo = 0;
  hi = n - 1;
  let end = n - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const rowTop = mid === 0 ? 0 : prefix[mid - 1];
    if (rowTop < bottom) {
      end = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }

  start = Math.max(0, start - overscan);
  end = Math.min(n - 1, end + overscan);
  return { start, end };
}

/** 生成 [start, end] 的索引数组（含两端） */
export function rangeIndices(start: number, end: number): number[] {
  const out: number[] = [];
  for (let i = start; i <= end; i++) out.push(i);
  return out;
}
