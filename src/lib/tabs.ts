import type { SessionTab } from "../composables/useCodex";
import type { EditorTab } from "../composables/useEditorTabs";

/** 标签类型（标签领域枚举；其它领域的 kind 不混入） */
export const TabKind = {
  Chat: "chat",
  File: "file",
  Diff: "diff",
  Preview: "preview",
  Terminal: "terminal",
  Commit: "commit",
} as const;
export type TabKind = (typeof TabKind)[keyof typeof TabKind];

/** 标签栏内置图标标识 */
export const TabIcon = {
  Chat: "chat",
  Terminal: "terminal",
  File: "file",
  Commit: "commit",
} as const;
export type TabIcon = (typeof TabIcon)[keyof typeof TabIcon];

/**
 * 所有标签（会话/文件/diff/预览/终端）的公共约束：
 * id/kind 身份、title 展示标题、icon 标签栏内置图标标识、workspace 工作区、
 * loading 异步加载态（会话加载线程/历史、文件/diff/预览读取内容、终端 spawn），
 * 与 isTabWorking（长任务运行中：会话回合/目标续跑、终端命令执行）区分。
 */
export interface EditorTabBase {
  id: string;
  kind: TabKind;
  title: string;
  /** 标签栏内置图标标识：chat=CODEX Logo，terminal=终端图标，file=文件图标（可叠加系统文件图标） */
  icon: TabIcon;
  workspace: string | null;
  loading: boolean;
}

/**
 * 统一“工作中”判定（标签角标与关闭守卫共用）：
 * - 会话标签：回合进行中或目标激活（后台自动续跑）；
 * - 终端标签：命令执行中且未退出、无错误；
 * - 文件/diff/预览：不参与（保存/加载为瞬态）。
 */
export function isTabWorking(tab: SessionTab | EditorTab): boolean {
  if (tab.kind === TabKind.Chat) {
    return tab.turnActive || tab.goalStatus === "active";
  }
  if (tab.kind === TabKind.Terminal) {
    return !!tab.busy && !tab.exited && !tab.error;
  }
  return false;
}
