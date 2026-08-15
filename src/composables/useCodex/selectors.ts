// useCodex 拆分模块：轻量选择器（原 useCodex.ts 的一部分，纯移动，行为不变）
import { permissionMode } from "../../lib/permissions";
import type { ThreadItem, ThreadSummary } from "../../lib/types";
import { store } from "./store";


export function permissionChip(): string {
  return permissionMode(store.permissionMode).chip;
}


export function currentItems(): ThreadItem[] {
  return store.currentThreadId ? (store.itemsByThread[store.currentThreadId] ?? []) : [];
}


export function threadTitle(t: ThreadSummary): string {
  return t.name || t.preview || "新会话";
}


export function currentOriginLabel(): string {
  if (store.currentThreadOrigin === "history") return "历史会话";
  if (store.currentThreadOrigin === "new") return "新会话";
  return "新会话";
}
