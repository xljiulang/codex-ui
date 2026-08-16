// useCodex 拆分模块：线程消息项与工作区解析（原 useCodex.ts 的一部分，纯移动，行为不变）
import { computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { ThreadItem, Turn } from "../../lib/types";
import { store } from "./store";
import type { SessionTab } from "./types";


export function isActiveItem(item: ThreadItem): boolean {
  return (
    item.streaming === true ||
    ["in_progress", "inProgress", "pending", "started"].includes(
      String(item.status ?? ""),
    )
  );
}


export function bumpActive(threadId: string, delta: number) {
  store.activeWorkByThread[threadId] = Math.max(
    0,
    (store.activeWorkByThread[threadId] ?? 0) + delta,
  );
}


export function upsertItem(threadId: string, item: ThreadItem) {
  const arr = (store.itemsByThread[threadId] ??= []);
  let idx = arr.findIndex((x) => x.id === item.id);
  if (idx < 0 && item.type === "userMessage" && item.clientId) {
    // 服务端推送的用户消息与本地乐观插入的通过 clientId 关联，避免重复
    idx = arr.findIndex(
      (x) => x.type === "userMessage" && x.clientId === item.clientId,
    );
  }
  const merged = idx >= 0 ? { ...arr[idx], ...item } : item;
  if (idx >= 0) {
    if (isActiveItem(arr[idx])) bumpActive(threadId, -1);
    arr[idx] = merged;
  } else {
    arr.push(item);
  }
  if (isActiveItem(merged)) bumpActive(threadId, 1);
  store.itemsRev++;
}


export function findItem(threadId: string, itemId: string): ThreadItem | undefined {
  return (store.itemsByThread[threadId] ?? []).find((x) => x.id === itemId);
}

/** 按 id 取消息项，不存在时用工厂创建并插入（流式 delta 场景） */
export function getOrCreateItem(
  threadId: string,
  itemId: string,
  factory: () => ThreadItem,
): ThreadItem {
  const existing = findItem(threadId, itemId);
  if (existing) return existing;
  const created = factory();
  upsertItem(threadId, created);
  return created;
}


export function flattenTurns(turns?: Turn[]): ThreadItem[] {
  if (!turns) return [];
  const out: ThreadItem[] = [];
  for (const t of turns) {
    out.push(...(t.items ?? []));
  }
  return out;
}


interface TurnsListPage {
  data: Turn[];
  nextCursor: string | null;
}


/** 用 thread/turns/list(itemsView=full) 拉取历史的完整工具/命令项 */
export async function loadFullItems(threadId: string): Promise<ThreadItem[] | null> {
  const turns: Turn[] = [];
  let cursor: string | null = null;
  try {
    for (let i = 0; i < 30; i++) {
      const res = (await invoke("codex_rpc", {
        method: "thread/turns/list",
        params: {
          threadId,
          cursor,
          limit: 50,
          // 协议 SortDirection 枚举为 "asc" | "desc"（"ascending" 会被服务端拒绝）
          sortDirection: "asc",
          itemsView: "full",
        },
      })) as TurnsListPage;
      turns.push(...(res.data ?? []));
      cursor = res.nextCursor ?? null;
      if (!cursor) break;
      if (turns.length > 2000) break; // 防超长会话
    }
  } catch (e) {
    // 服务端不支持时回退到 thread_read 的摘要项；记录原因便于排查协议漂移
    console.warn("[codex-ui] thread/turns/list(full) 失败，回退摘要加载:", e);
    return null;
  }
  return flattenTurns(turns);
}


/**
 * 会话工作区解析（所有入口共用，避免优先级不一致）：
 * 有会话时以会话工作区为准（忽略残留的 newChatWorkspace），无会话（新建会话中）
 * 优先待新建目录，其次启动工作区；均跳过空串。
 * 传 tab 时按指定会话标签解析（后台标签发送回合时沙箱可写根等应跟随该标签）。
 */
export function resolveSessionWorkspace(tab?: SessionTab): string {
  const cwd = tab
    ? tab.threadId
      ? tab.workspace
      : tab.newChatWorkspace
    : store.currentThreadId
      ? store.currentThreadWorkspace
      : store.newChatWorkspace;
  return cwd?.trim() || store.server.startupWorkspace?.trim() || "";
}


/**
 * 当前工作区：由活动编辑器标签决定（文件/diff/预览/终端标签由 EditorPane 写入
 * store.workspace），null 或未设置时回落会话工作区。资源/Git 面板与新建会话初始目录跟随它。
 */
export const workspace = computed(
  () => store.workspace ?? resolveSessionWorkspace(),
);
