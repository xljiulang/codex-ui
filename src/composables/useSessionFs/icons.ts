import { invoke } from "@tauri-apps/api/core";
import { workspace } from "../useCodex";
import type {
  FsEntry,
  IconRequest,
  IconResult,
} from "../../lib/sessionFs";
import {
  BUILTIN_ICON_NAMES,
  builtinFileIcon,
} from "../../lib/fileTypeIcons";
import { iconCache } from "./state";

/** 图标缓存上限：超限按插入顺序淘汰最旧 */
const ICON_CACHE_MAX = 1000;

/** 内置图标覆盖的特殊文件名集合（Dockerfile/Makefile/.gitignore/.env 等） */
const SPECIAL_ICON_NAMES = new Set(BUILTIN_ICON_NAMES);

/** 图标缓存键：目录返回 null（不在范围）；特殊文件名/按扩展名共享，其余按路径 */
export function iconCacheKey(entry: FsEntry): string | null {
  if (entry.isDir) return null;
  const lowerName = entry.name.toLowerCase();
  if (SPECIAL_ICON_NAMES.has(lowerName)) return `ext:${lowerName}`;
  const dot = entry.name.lastIndexOf(".");
  if (dot > 0 && dot < entry.name.length - 1) {
    return `ext:${entry.name.slice(dot).toLowerCase()}`;
  }
  return `file:${entry.relPath}`;
}

/** 读取缓存图标：未缓存或目录返回 null（渲染层回退 SVG） */
export function iconFor(entry: FsEntry): string | null {
  const key = iconCacheKey(entry);
  return key ? (iconCache.get(key) ?? null) : null;
}

function setIconCache(key: string, value: string | null) {
  iconCache.delete(key); // 重新插入，保持 LRU 顺序
  iconCache.set(key, value);
  if (iconCache.size > ICON_CACHE_MAX) {
    const oldest = iconCache.keys().next().value;
    if (oldest !== undefined) iconCache.delete(oldest);
  }
}

/**
 * 按可见行懒加载缺失的文件图标：同扩展名只发一个代表路径，结果回填缓存；
 * 单个失败缓存 null 不重试；整批失败（如 root 无效）不缓存，待下次可见行变化重试。
 */
export async function ensureEntryIcons(
  entries: FsEntry[],
  root: string = workspace.value,
): Promise<void> {
  if (!root || !entries.length) return;
  // 内置代码文件图标优先：命中即写缓存，不再发起系统图标请求
  for (const e of entries) {
    const key = iconCacheKey(e);
    if (!key || iconCache.has(key)) continue;
    const builtin = builtinFileIcon(e.name);
    if (builtin) setIconCache(key, builtin);
  }
  const byKey = new Map<string, FsEntry>();
  for (const e of entries) {
    const key = iconCacheKey(e);
    if (key && !iconCache.has(key) && !byKey.has(key)) byKey.set(key, e);
  }
  if (!byKey.size) return;

  const pathToKey = new Map<string, string>();
  const requests: IconRequest[] = [];
  for (const e of byKey.values()) {
    pathToKey.set(e.path, iconCacheKey(e)!);
    requests.push({ path: e.path });
  }
  try {
    const results = await invoke<IconResult[]>("session_fs_icons", {
      workspace: root,
      requests,
    });
    for (const r of results) {
      const key = pathToKey.get(r.path);
      if (key) setIconCache(key, r.dataUri);
    }
    // 请求了但未返回的键置 null，避免反复请求
    for (const key of byKey.keys()) {
      if (!iconCache.has(key)) setIconCache(key, null);
    }
  } catch {
    // 整批失败：保持未缓存状态，稍后重试
  }
}

/** 「新建文本文件」菜单系统图标任务：防止重复发起取图请求 */
let textFileIconTask: Promise<void> | null = null;

/** 按 .txt 扩展名取一次系统图标并写入缓存（复用 ext:.txt 键，.txt 文件行/标签同步受益） */
export async function ensureTextFileIcon(): Promise<void> {
  const root = workspace.value;
  if (!root || iconCache.has("ext:.txt")) return;
  if (textFileIconTask) return textFileIconTask;
  textFileIconTask = (async () => {
    try {
      const uri = await invoke<string | null>("session_fs_icon_for_ext", {
        ext: ".txt",
      });
      setIconCache("ext:.txt", uri ?? null);
    } catch {
      // 取不到/失败：缓存 null 不再重试，菜单回退内置 SVG
      setIconCache("ext:.txt", null);
    } finally {
      textFileIconTask = null;
    }
  })();
  return textFileIconTask;
}

/** 右键菜单「新建文本文件」图标：缓存未就绪/不可用时返回 undefined（调用方回退 SVG） */
export function textFileMenuIcon(): string | undefined {
  return iconCache.get("ext:.txt") ?? undefined;
}
