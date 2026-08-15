// useCodex 拆分模块：置顶/标题总结能力探测（原 useCodex.ts 的一部分，纯移动，行为不变）
import { invoke } from "@tauri-apps/api/core";
import type { ThreadSummary } from "../../lib/types";


/** 置顶协议能力（由 codex_pin_capability 探测，覆盖三代 codex 协议） */
export type PinProtocol =
  | "section_move" // 新版：threadSection/move { sectionId }
  | "metadata_section" // 分区时代：thread/metadata/update { sectionId }
  | "metadata_is_pinned" // 旧版：thread/metadata/update { isPinned }
  | "unsupported";


export interface PinCapability {
  protocol: PinProtocol;
  pinnedSectionId: string | null;
  /** section_move 时实际可用的分区移动方法名（新版 codex 为 thread/section/move） */
  sectionMoveMethod?: "threadSection/move" | "thread/section/move";
}


/** 新版协议内置的 “Pinned” 分区，作为置顶的持久化位置（可用 threadSection/list 发现） */
const PINNED_SECTION_NAME = "Pinned";

/** 兜底：该内置分区 id 在所有安装中固定（与 codex 源码常量一致） */
const FALLBACK_PINNED_SECTION_ID = "01984de2-8f74-7c91-a3b2-5c5e937cf318";

let pinCapabilityCache: PinCapability | null = null;


/** 仅测试用：重置置顶能力缓存 */
export function __resetPinnedSectionForTest() {
  pinCapabilityCache = null;
}


/** 线程是否置顶：优先看服务端返回的 section 是否指向内置 Pinned 分区 */
function isPinnedThread(t: ThreadSummary): boolean {
  if (t.isPinned) return true;
  const sid = t.section?.id;
  if (!sid) return false;
  return (
    sid === pinCapabilityCache?.pinnedSectionId ||
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


/** 获取当前 codex 的置顶协议能力（首次调用后缓存；探测失败返回 null 以便重试） */
export async function getPinCapability(): Promise<PinCapability | null> {
  if (pinCapabilityCache) return pinCapabilityCache;
  try {
    const cap = await invoke<PinCapability>("codex_pin_capability");
    pinCapabilityCache = cap;
    return cap;
  } catch {
    return null;
  }
}


/** 标题总结能力（codex_title_helper_capability 探测结果，覆盖版本差异） */
export interface TitleHelperCapability {
  experimentalApi: boolean;
  ephemeral: boolean;
}


let titleHelperCapabilityCache: TitleHelperCapability | null = null;


/** 仅测试用：重置标题总结能力缓存 */
export function __resetTitleHelperCapabilityForTest() {
  titleHelperCapabilityCache = null;
}


/** 获取当前 codex 的标题总结能力（首次调用后缓存；探测失败返回 null） */
export async function getTitleHelperCapability(): Promise<TitleHelperCapability | null> {
  if (titleHelperCapabilityCache) return titleHelperCapabilityCache;
  try {
    const cap = await invoke<TitleHelperCapability>("codex_title_helper_capability");
    titleHelperCapabilityCache = cap;
    return cap;
  } catch {
    return null;
  }
}
