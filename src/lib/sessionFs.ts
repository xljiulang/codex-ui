import { normalizePathKey as normalizePathKeyForMerge } from "./path";
// 路径工具统一实现收敛到 ./path：本文件仅 re-export，保持既有调用方不变
export {
  dirNameOf,
  isPathUnderRoot,
  joinFsPath,
  normalizeFsPath,
  normalizePathKey,
  pathEquals,
  relPathOf,
} from "./path";

/** 会话资源条目（Rust session_fs 命令返回，字段 camelCase） */
export interface FsEntry {
  name: string;
  path: string;
  /** 相对工作目录的路径（. 表示根） */
  relPath: string;
  isDir: boolean;
  /** 文件字节数；目录为 null */
  size: number | null;
  modifiedAtMs: number;
  createdAtMs: number;
  /** 目录的直接可见子项数；文件为 null */
  childCount: number | null;
}

/** 捆绑 rg 可用状态（Rust session_fs_rg_status 返回，字段 camelCase） */
export interface RgStatus {
  available: boolean;
  path: string | null;
}

/** rg 内容命中（每个文件只返回首个命中行） */
export interface RgHit {
  entry: FsEntry;
  lineNumber: number;
  lineText: string;
}

/** rg 内容搜索结果 */
export interface RgSearchResult {
  available: boolean;
  hits: RgHit[];
}

/** 搜索命中行摘要：行号 + 命中行文本 */
export interface SearchSnippet {
  lineNumber: number;
  text: string;
}

export interface SearchMergeResult {
  results: FsEntry[];
  snippets: Record<string, SearchSnippet>;
}

/**
 * 合并文件名搜索与 rg 内容搜索结果：
 * - 文件名结果保持原顺序在前；
 * - 内容-only 文件按相对路径排序后追加；
 * - 同一文件同时命中时保留文件名结果位置，并附带首个命中行摘要。
 */
export function mergeSearchResults(
  nameHits: FsEntry[],
  contentHits: RgHit[],
): SearchMergeResult {
  const results: FsEntry[] = [];
  const snippets: Record<string, SearchSnippet> = {};
  const seen = new Set<string>();
  for (const entry of nameHits) {
    const key = normalizePathKeyForMerge(entry.path);
    if (seen.has(key)) continue;
    seen.add(key);
    results.push(entry);
  }
  const contentOnly: FsEntry[] = [];
  for (const hit of contentHits) {
    const key = normalizePathKeyForMerge(hit.entry.path);
    if (!snippets[key]) {
      snippets[key] = { lineNumber: hit.lineNumber, text: hit.lineText };
    }
    if (seen.has(key)) continue;
    seen.add(key);
    contentOnly.push(hit.entry);
  }
  contentOnly.sort((a, b) => {
    const al = a.relPath.toLowerCase();
    const bl = b.relPath.toLowerCase();
    if (al < bl) return -1;
    if (al > bl) return 1;
    return a.relPath < b.relPath ? -1 : a.relPath > b.relPath ? 1 : 0;
  });
  results.push(...contentOnly);
  return { results, snippets };
}

export type ResourceRow =
  | { kind: "root"; entry: FsEntry; collapsed: boolean; depth: 0 }
  | { kind: "dir"; entry: FsEntry; depth: number; collapsed: boolean }
  | { kind: "file"; entry: FsEntry; depth: number };

/** 系统图标请求（session_fs_icons 命令入参，仅文件；camelCase 直传） */
export interface IconRequest {
  path: string;
}

/** 系统图标结果：dataUri 为空表示该文件取不到系统图标（前端回退 SVG） */
export interface IconResult {
  path: string;
  dataUri: string | null;
}

/** 文件大小人类可读格式：B / KB / MB / GB / TB */
export function formatFileSize(bytes: number | null): string {
  if (bytes == null || bytes < 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes;
  let u = -1;
  do {
    v /= 1024;
    u++;
  } while (v >= 1024 && u < units.length - 1);
  const num = v >= 100 ? Math.round(v).toString() : v.toFixed(1);
  return `${num} ${units[u]}`;
}

/**
 * 把树状缓存展平为渲染行：
 * - 根行始终显示（depth 0）；
 * - 只展开 expanded 集合中的目录，子目录递归展平；
 * - 尚未加载的目录不产生子行（懒加载）。
 */
export function flattenResourceTree(
  root: FsEntry,
  childrenByPath: Record<string, FsEntry[]>,
  expanded: Set<string>,
): ResourceRow[] {
  const rows: ResourceRow[] = [];
  const rootExpanded = expanded.has(root.path);
  rows.push({ kind: "root", entry: root, collapsed: !rootExpanded, depth: 0 });
  if (!rootExpanded) return rows;

  const walk = (entry: FsEntry, depth: number) => {
    for (const child of childrenByPath[entry.path] ?? []) {
      if (child.isDir) {
        const collapsed = !expanded.has(child.path);
        rows.push({ kind: "dir", entry: child, depth, collapsed });
        if (!collapsed) walk(child, depth + 1);
      } else {
        rows.push({ kind: "file", entry: child, depth });
      }
    }
  };
  walk(root, 1);
  return rows;
}
