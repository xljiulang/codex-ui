import { watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { setToast, toastError, workspace } from "../useCodex";
import { normalizeFsPath } from "../../lib/path";
import {
  fileSideMode,
  pruneTreeToRoot,
  rootError,
  rootEntry,
  resetTree,
} from "./state";
import {
  ensureRootLoaded,
  refreshAll,
} from "./tree";
import { ensureTextFileIcon } from "./icons";
import { clearSearch, resetSearchState } from "./search";
import { refreshTabsFromFs } from "../useEditorTabs";

let active = false;
/** 已加载的根路径：同根重新激活时保留展开状态，仅刷新数据 */
let loadedRoot = "";
let unlistenFsEvent: (() => void) | null = null;
let watcherStarted = false;

/** 文件监听跟随工作区（始终生效，不依赖资源面板激活）：有工作区即监听 */
async function syncWatcher() {
  const root = normalizeFsPath(workspace.value);
  if (root) {
    if (!unlistenFsEvent) {
      try {
        unlistenFsEvent = await listen("session-fs/changed", (e) => {
          if (active) void refreshAll();
          const payload = e.payload as
            | { root?: unknown; paths?: unknown }
            | undefined;
          void refreshTabsFromFs(
            payload &&
              typeof payload.root === "string" &&
              Array.isArray(payload.paths) &&
              payload.paths.every((p) => typeof p === "string")
              ? {
                  root: payload.root,
                  paths: payload.paths as string[],
                }
              : undefined,
          );
        });
      } catch {
        // 非 Tauri 环境（如单测）忽略
      }
    }
    try {
      await invoke("session_fs_watch_start", { workspace: root });
      watcherStarted = true;
    } catch (e) {
      setToast(toastError(e));
    }
  } else {
    unlistenFsEvent?.();
    unlistenFsEvent = null;
    if (watcherStarted) {
      try {
        await invoke("session_fs_watch_stop");
      } catch {
        // 忽略停止失败
      }
      watcherStarted = false;
    }
  }
}

/** 激活资源 Tab：解析工作区后加载树；无确定工作区时重置为空态（不加载/监听） */
async function activate() {
  const root = normalizeFsPath(workspace.value);
  if (!root) {
    resetTree();
    resetSearchState();
    rootError.value = "暂无工作目录";
    return;
  }
  // 预取「新建文本文件」菜单的系统 .txt 图标（写 ext:.txt 缓存）
  void ensureTextFileIcon();
  if (loadedRoot !== root) {
    loadedRoot = root;
    // 不 resetTree：旧数据保持到新根加载完成（首屏无旧数据，行为等价）
    const ok = await ensureRootLoaded(root);
    if (ok) {
      pruneTreeToRoot(root);
      clearSearch();
    } else if (rootEntry.value && rootError.value) {
      setToast(rootError.value);
    }
  } else {
    await refreshAll();
  }
}

/** 会话资源 Tab 激活状态：监听常驻（由 syncWatcher 跟随工作区），激活时额外加载/刷新树 */
export function setSessionFsActive(v: boolean) {
  if (active === v) return;
  active = v;
  void syncWatcher();
  if (v) void activate();
}

// 根目录切换（切换会话/新建会话选目录）：不 resetTree，旧数据保持到新根加载完成，
// 成功后替换并清理旧根缓存；监听跟随新根
watch(workspace, async (r, old) => {
  const root = normalizeFsPath(r ?? "");
  const prev = normalizeFsPath(old ?? "");
  if (root === prev) return;
  void syncWatcher();
  if (!active) return;
  if (root) {
    if (loadedRoot !== root) {
      loadedRoot = root;
      const ok = await ensureRootLoaded(root);
      if (ok) {
        pruneTreeToRoot(root);
        clearSearch();
      } else if (rootEntry.value && rootError.value) {
        setToast(rootError.value);
      }
    }
  } else {
    resetTree();
    resetSearchState();
    rootError.value = "暂无工作目录";
  }
});

/** 仅测试用：清空模块状态 */
export function __resetSessionFsForTest() {
  active = false;
  loadedRoot = "";
  // 显示态是用户偏好（根切换不重置），仅测试重置回默认
  fileSideMode.value = "time";
  resetTree();
  resetSearchState();
  unlistenFsEvent?.();
  unlistenFsEvent = null;
  watcherStarted = false;
}
