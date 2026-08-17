// useEditorTabs 拆分后的统一出口。
// 原 useEditorTabs.ts（545 行）已按领域拆分为 useEditorTabs/ 目录；
// 本文件仅做再导出，保证调用方 `import { ... } from "../useEditorTabs"` 完全不变。
export { activeTab, activeTabId, activateTab, tabs } from "../useTabs";
export type {
  CommitEditorTab,
  DocxEditorTab,
  DiffEditorTab,
  DiffPreviewParams,
  EditorTab,
  FileEditorTab,
  PreviewEditorTab,
  TerminalEditorTab,
} from "./types";
export {
  isFileTabOpen,
  isTerminalBusy,
  openCommitFileDiffTab,
  openCommitTab,
  openDocxTab,
  openDiffTab,
  openFileTab,
  openPreviewTab,
  saveDocxTab,
  openTerminalTab,
  saveFileTab,
} from "./open";
export {
  __resetEditorTabsForTest,
  cancelClose,
  closeAllOtherTabs,
  closeAllTabs,
  closeAnyTab,
  closeTab,
  closeTabsToLeftAll,
  closeTabsToRightAll,
  dirtyEditableTabs,
  discardTabAndClose,
  pendingCloseId,
  saveAllDirtyTabs,
  saveTabAndClose,
} from "./close";
