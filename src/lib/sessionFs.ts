import { formatRelativeTime } from "./format";

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

/** 文件时间列展示：复用历史会话的相对时间格式 */
export function formatFileTime(ms: number): string {
  if (!ms) return "";
  return formatRelativeTime(ms / 1000);
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

/** Windows 路径拼接：去尾分隔符后追加反斜杠 */
export function joinFsPath(base: string, name: string): string {
  return base.replace(/[\\/]+$/, "") + "\\" + name;
}

