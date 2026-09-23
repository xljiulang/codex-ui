// useCodex 拆分模块：轻量选择器（原 useCodex.ts 的一部分，纯移动，行为不变）
import { permissionMode } from "../../lib/permissions";
import type { ThreadItem, ThreadSummary } from "../../lib/types";
import { activeSessionTab } from "./sessionState";
import { store } from "./store";

export function permissionChip(): string {
  return permissionMode(
    activeSessionTab()?.permissionMode ?? "ask-for-approval",
  ).chip;
}

export function currentItems(): ThreadItem[] {
  const tid = activeSessionTab()?.threadId;
  return tid ? (store.itemsByThread[tid] ?? []) : [];
}

export function threadTitle(t: ThreadSummary): string {
  return t.name || t.preview || "新会话";
}

export function currentOriginLabel(): string {
  if (activeSessionTab()?.origin === "history") return "历史会话";
  return "新会话";
}
