import { computed, ref, shallowReactive } from "vue";
import { TabKind } from "../lib/tabs";
import type { SessionTab } from "./useCodex";
import type { EditorTab } from "./useEditorTabs";

/** 统一标签：会话标签与文件/diff/预览/终端标签同列表维护 */
export type Tab = SessionTab | EditorTab;

/** 标签 kind 排位：chat=0、terminal=1、其它=2、settings=3（统一列表四块顺序，设置恒在最后） */
function kindRank(kind: Tab["kind"]): number {
  if (kind === TabKind.Chat) return 0;
  if (kind === TabKind.Terminal) return 1;
  if (kind === TabKind.Settings) return 3;
  return 2;
}

/**
 * 统一标签列表：会话标签在前、终端标签居中、文件/diff/预览按打开顺序在后。
 * 由 useTabs 统一持有，useEditorTabs（编辑器/终端）与 useCodex（会话）都操作这份列表。
 * 用浅响应式：标签对象自身由各创建方用 reactive() 包装（编辑器/终端已如此），
 * 避免深 unwrap 破坏 EditorState 等类类型的类型同一性。
 */
export const tabs = shallowReactive<Tab[]>([]);

/** 当前活动标签 id（会话/文件/diff/预览/终端统一） */
export const activeTabId = ref("");

/** 当前活动标签 */
export const activeTab = computed<Tab | null>(
  () => tabs.find((t) => t.id === activeTabId.value) ?? null,
);

/** 按 kind 排位插入：插入到列表中第一个排位更高的标签之前（否则追加末尾） */
export function insertTab(tab: Tab): void {
  const rank = kindRank(tab.kind);
  const idx = tabs.findIndex((t) => kindRank(t.kind) > rank);
  if (idx < 0) tabs.push(tab);
  else tabs.splice(idx, 0, tab);
}

/** 激活标签：id 存在于列表才生效 */
export function activateTab(id: string): void {
  if (tabs.some((t) => t.id === id)) activeTabId.value = id;
}

/** 按 id 查找标签 */
export function findTab(id: string): Tab | undefined {
  return tabs.find((t) => t.id === id);
}

/** 测试专用：清空统一标签列表 */
export function __resetTabsForTest(): void {
  tabs.splice(0, tabs.length);
  activeTabId.value = "";
}
