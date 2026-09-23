// useCodex 拆分模块：置顶分区 id 与历史排序（仅适配 codex 0.149.x：
// 置顶固定使用 threadSection/list + thread/section/move，不再做能力探测）
import { invoke } from "@tauri-apps/api/core";
import type { ThreadSummary } from "../../lib/types";

/** 内置 “Pinned” 分区名（可用 threadSection/list 发现） */
const PINNED_SECTION_NAME = "Pinned";

/** 兜底：该内置分区 id 在所有安装中固定（与 codex 源码常量一致） */
const FALLBACK_PINNED_SECTION_ID = "01984de2-8f74-7c91-a3b2-5c5e937cf318";

let pinnedSectionIdCache: string | null = null;

/** 仅测试用：重置置顶分区 id 缓存 */
export function __resetPinnedSectionForTest() {
  pinnedSectionIdCache = null;
}

/** 线程是否置顶：优先看服务端返回的 section 是否指向内置 Pinned 分区 */
function isPinnedThread(t: ThreadSummary): boolean {
  if (t.isPinned) return true;
  const sid = t.section?.id;
  if (!sid) return false;
  return (
    sid === pinnedSectionIdCache ||
    sid === FALLBACK_PINNED_SECTION_ID ||
    t.section?.name === PINNED_SECTION_NAME
  );
}

/** 把服务端 section 状态物化为 isPinned 字段，供现有 UI 与排序直接使用 */
function normalizeThreadPins(list: ThreadSummary[]): ThreadSummary[] {
  return list.map((t) => ({ ...t, isPinned: isPinnedThread(t) }));
}

/** 固定优先，再按最近时间降序 */
export function sortThreads(list: ThreadSummary[]): ThreadSummary[] {
  return normalizeThreadPins(list).sort((a, b) => {
    if (a.isPinned !== b.isPinned) return a.isPinned ? -1 : 1;
    const ar = a.recencyAt ?? a.updatedAt ?? 0;
    const br = b.recencyAt ?? b.updatedAt ?? 0;
    return br - ar;
  });
}

/** 获取当前 codex 的内置 Pinned 分区 id（首次调用后缓存；失败返回 null 以便重试） */
export async function getPinnedSectionId(): Promise<string | null> {
  if (pinnedSectionIdCache) return pinnedSectionIdCache;
  try {
    const id = await invoke<string>("codex_pinned_section_id");
    pinnedSectionIdCache = id;
    return id;
  } catch {
    return null;
  }
}
