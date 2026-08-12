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

/** 会话排序：置顶优先，其余按最近时间倒序 */
function byPinThenRecency(a: ThreadSummary, b: ThreadSummary): number {
  if (!!a.isPinned !== !!b.isPinned) return a.isPinned ? -1 : 1;
  return recencyOf(b) - recencyOf(a);
}

/**
 * 把已排序的会话列表按 cwd 分组为目录行 + 平铺行：
 * - 相同 cwd 归入同一目录（仅 1 条也建目录）；
 * - cwd 缺失/空串的会话保持平铺，排在所有目录之后；
 * - 目录行按目录名 A-Z（忽略大小写、数字自然序），同名按规范化 key 兜底；
 * - 目录内与平铺会话均按置顶优先 + 最近时间倒序。
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

  for (const group of groups.values()) {
    group.threads.sort(byPinThenRecency);
  }

  const folderRows: HistoryRow[] = [...groups.values()]
    .sort((a, b) => {
      const byLabel = a.label.localeCompare(b.label, undefined, {
        numeric: true,
        sensitivity: "base",
      });
      return byLabel || a.key.localeCompare(b.key);
    })
    .map((group): HistoryRow => ({ kind: "group", group }));

  singles.sort(byPinThenRecency);
  const singleRows: HistoryRow[] = singles.map(
    (thread): HistoryRow => ({ kind: "item", thread }),
  );

  return [...folderRows, ...singleRows];
}
