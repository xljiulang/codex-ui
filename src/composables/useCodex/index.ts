// useCodex 拆分后的统一出口。
// 原 useCodex.ts（2610 行）已按领域拆分为 useCodex/ 目录下多个模块；
// 本文件仅做再导出，保证调用方 `import { ... } from "../useCodex"` 完全不变。
export { store } from "./store";
export { isGoalStatus } from "./types";
export type {
  ConfirmRequest,
  GoalStatus,
  PanelTab,
  PlanPrompt,
  PluginCatalogItem,
  PluginItem,
  PluginMarketplaceInfo,
  PluginMarketplaceLoadError,
  SessionTab,
  SkillItem,
} from "./types";
export { setToast, toastError } from "./toast";
export { askConfirm, settleConfirm } from "./confirm";
export {
  __resetPinnedSectionForTest,
  getPinnedSectionId,
  sortThreads,
} from "./capabilities";
export { resolveSessionWorkspace, workspace } from "./items";
export { currentItems, currentOriginLabel, permissionChip, threadTitle } from "./selectors";
export {
  __resetSessionTabsForTest,
  activeSessionTab,
  findSessionTabByThread,
  isThreadOpen,
  isThreadRunning,
  sessionTabTitle,
} from "./sessionState";
export { setWindowBaseTitle, updateWindowTitle } from "./windowTitle";
export {
  currentModelId,
  effectiveEffort,
  ensureSkills,
  ensureThreadPlugins,
  loadModels,
  loadSettings,
  modelDisplayName,
  refreshServer,
  saveSettings,
} from "./settings";
export {
  addMarketplace,
  installPlugin,
  isAuthRequiredError,
  loadPluginCatalog,
  refreshPluginCaches,
  removeMarketplace,
  uninstallPlugin,
} from "./plugins";
export { loadMcpServers, saveMcpServers } from "./mcp";
export {
  ensureWeChatEvents,
  isThreadBound,
  bindingOfThread,
  refreshWeChatState,
  wechatBindLoginStart,
  wechatUnbind,
} from "./wechat";
export {
  loadModelProviderConfig,
  saveModelProviderConfig,
  type ModelProviderConfigState,
} from "./modelProviderConfig";
export {
  autoTitleThread,
  clearSearch,
  refreshThreads,
  renameThread,
  sanitizeTitle,
  searchThreads,
  togglePin,
} from "./threads";
export { clearGoal, interrupt, setGoal } from "./turnControl";
export {
  addAttachmentToActiveSession,
  closeAllSessionTabs,
  closeSessionTab,
  registerComposerAddHandler,
  switchSessionTab,
  unregisterComposerAddHandler,
} from "./sessionTabs";
export {
  deleteThread,
  dismissPlanPrompt,
  executePlan,
  exitPlanMode,
  newEmptyChat,
  openHistorySession,
  openNewSession,
  openSessionTabForThread,
  openThread,
  pickAndOpenNewSession,
  pickingNewSessionDir,
  respondInteraction,
  sendPrompt,
} from "./actions";
export { disposeEvents, wireEvents } from "./events";
export { init } from "./boot";
