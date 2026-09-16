// useSessionFs 拆分后的统一出口。
// 原 useSessionFs.ts（665 行）已按领域拆分为 useSessionFs/ 目录下多个模块；
// 本文件仅做再导出，保证调用方 `import { ... } from "../useSessionFs"` 完全不变。
export {
  childrenByPath,
  expanded,
  fileSideMode,
  iconCache,
  loadingByPath,
  loadingRoot,
  rootEntry,
  rootError,
  searchActive,
  searchResults,
  searchSnippets,
  searchTerm,
  searching,
  selectedPath,
  treeRows,
  pruneDeadDir,
  toggleFileSideMode,
} from "./state";
export {
  ensureEntryIcons,
  ensureTextFileIcon,
  iconCacheKey,
  iconFor,
  textFileMenuIcon,
} from "./icons";
export {
  loadDir,
  refreshAll,
  revealAbsPathInTree,
  revealActiveTab,
  revealInTree,
  toggleDir,
} from "./tree";
export { clearSearch, onSearchInput, runSearchNow, searchSnippet } from "./search";
export {
  copyEntry,
  createFolder,
  createTextFile,
  deleteEntry,
  moveEntry,
  pasteAvailable,
  pasteInto,
  renameEntry,
  revealInExplorer,
} from "./fileOps";
export {
  addAsAttachment,
  openAudioPreview,
  openDocxPreview,
  openImagePreview,
  openPathInApp,
  openPdfPreview,
  openPptxPreview,
  openTextEditor,
  openVideoPreview,
  openXlsxPreview,
  probeTextEntry,
} from "./open";
export { setSessionFsActive, __resetSessionFsForTest } from "./watcher";
