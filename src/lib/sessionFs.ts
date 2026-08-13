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

/** 视为文本/代码文件、可应用内预览的扩展名（小写，含 highlight.ts 全部代码扩展名） */
const TEXT_EXTENSIONS = new Set([
  // 代码（与 highlight.ts EXT_TO_LANG 对齐）
  "ts", "mts", "cts", "js", "jsx", "mjs", "cjs", "py", "rs", "cs",
  "c", "h", "cpp", "cc", "cxx", "hpp", "go", "java", "kt", "kts",
  "json", "md", "markdown", "yml", "yaml", "sh", "bash", "zsh",
  "ps1", "psm1", "sql", "css", "html", "htm", "xml", "svg", "ini",
  "cfg", "php", "rb", "diff", "patch",
  // 纯文本 / 文档 / 配置
  "txt", "text", "log", "csv", "tsv", "toml", "conf", "env",
  "scss", "less", "vue", "svelte", "astro", "graphql", "proto",
  "prisma", "tf", "gradle", "lock", "properties", "editorconfig",
  "npmrc", "yarnrc", "babelrc", "eslintrc", "prettierrc",
  "gitignore", "gitattributes", "dockerignore", "gitmodules",
]);

/** 无扩展名/点文件也视为文本（小写匹配） */
const TEXT_FILE_NAMES = new Set([
  "dockerfile", "makefile", "license", "readme", "env",
  "gitignore", "gitattributes", "npmrc", "yarnrc", "editorconfig",
  "babelrc", "eslintrc", "prettierrc", "gitmodules", "gitkeep",
]);

/** 是否为文本/代码文件（扩展名或文件名白名单，大小写不敏感） */
export function isTextFile(name: string): boolean {
  const base = name.trim();
  if (!base) return false;
  const lower = base.toLowerCase();
  if (
    TEXT_FILE_NAMES.has(lower) ||
    TEXT_FILE_NAMES.has(lower.replace(/^\./, ""))
  ) {
    return true;
  }
  const dot = lower.lastIndexOf(".");
  if (dot <= 0) return false;
  return TEXT_EXTENSIONS.has(lower.slice(dot + 1));
}
