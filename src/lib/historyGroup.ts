import type { ThreadSummary } from "./types";

/** 历史目录分组（key 为规范化路径，path 为原始完整路径用于 tooltip） */
export interface HistoryGroup {
  key: string;
  path: string;
  label: string;
  threads: ThreadSummary[];
}

export type HistoryRow =
  | { kind: "group"; group: HistoryGroup }
  | { kind: "item"; thread: ThreadSummary };

/** 规范化目录分组键：统一反斜杠、去尾部分隔符；Windows 下大小写不敏感 */
export function normalizeDirKey(cwd: string): string {
  return cwd.replace(/[\\/]+$/, "").replace(/\//g, "\\").toLowerCase();
}

/** 目录显示名：路径最后一段；根路径（如 C:\）原样返回 */
export function dirLabel(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const parts = trimmed.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 1) return path;
  return parts[parts.length - 1];
}

function recencyOf(t: ThreadSummary): number {
  return t.recencyAt ?? t.updatedAt ?? 0;
}

function groupRecency(group: HistoryGroup): number {
  return group.threads.reduce((max, t) => Math.max(max, recencyOf(t)), 0);
}

/**
 * 把已排序的会话列表按 cwd 分组为目录行 + 平铺行：
 * - 相同 cwd 归入同一目录（仅 1 条也建目录）；
 * - cwd 缺失/空串的会话保持平铺；
 * - 顶层排序：含置顶会话的目录/条目排最前，其余按最近时间降序；
 * - 组内顺序沿用入参顺序（外部已按置顶优先 + 最近时间排序）。
 */
export function groupThreads(list: ThreadSummary[]): HistoryRow[] {
  const groups = new Map<string, HistoryGroup>();
  const singles: ThreadSummary[] = [];

  for (const t of list) {
    const cwd = t.cwd?.trim();
    if (!cwd) {
      singles.push(t);
      continue;
    }
    const key = normalizeDirKey(cwd);
    let group = groups.get(key);
    if (!group) {
      group = { key, path: cwd, label: dirLabel(cwd), threads: [] };
      groups.set(key, group);
    }
    group.threads.push(t);
  }

  const rows: HistoryRow[] = [
    ...[...groups.values()].map((group): HistoryRow => ({ kind: "group", group })),
    ...singles.map((thread): HistoryRow => ({ kind: "item", thread })),
  ];

  return rows.sort((a, b) => {
    const aPinned =
      a.kind === "group"
        ? a.group.threads.some((t) => !!t.isPinned)
        : !!a.thread.isPinned;
    const bPinned =
      b.kind === "group"
        ? b.group.threads.some((t) => !!t.isPinned)
        : !!b.thread.isPinned;
    if (aPinned !== bPinned) return aPinned ? -1 : 1;
    const ar = a.kind === "group" ? groupRecency(a.group) : recencyOf(a.thread);
    const br = b.kind === "group" ? groupRecency(b.group) : recencyOf(b.thread);
    return br - ar;
  });
}
