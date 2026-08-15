import { watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { setToast, store, toastError, workspace } from "../useCodex";
import {
  rootError,
  resetTree,
} from "./state";
import {
  loadRoot,
  refreshAll,
  resolveFallbackRoot,
} from "./tree";
import { ensureTextFileIcon } from "./icons";
import { resetSearchState } from "./search";

let active = false;
/** 已加载的根路径：同根重新激活时保留展开状态，仅刷新数据 */
let loadedRoot = "";
let unlistenFsEvent: (() => void) | null = null;
let watcherStarted = false;

async function syncWatcher() {
  const root = workspace.value;
  if (active && root) {
    if (!unlistenFsEvent) {
      try {
        unlistenFsEvent = await listen("session-fs/changed", () => {
          if (active) void refreshAll();
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

/** 激活资源 Tab：解析工作区（含 startup_workspace 兜底）后加载树 */
async function activate() {
  const root = workspace.value || (await resolveFallbackRoot());
  if (!root) {
    rootError.value = "暂无工作目录";
    return;
  }
  if (!workspace.value) {
    // 兜底结果回写全局，让头部等其它读取点保持一致
    store.server.startupWorkspace = root;
  }
  // 预取「新建文本文件」菜单的系统 .txt 图标（写 ext:.txt 缓存）
  void ensureTextFileIcon();
  if (loadedRoot !== root) {
    loadedRoot = root;
    resetTree();
    await loadRoot(root);
  } else {
    await refreshAll();
  }
}

/** 会话资源 Tab 激活状态：激活时启动监听并加载树，切走时停止监听 */
export function setSessionFsActive(v: boolean) {
  if (active === v) return;
  active = v;
  void syncWatcher();
  if (v) void activate();
}

// 根目录切换（切换会话/新建会话选目录）：重置并重新加载，监听跟随新根
watch(workspace, (r, old) => {
  if (r === old) return;
  if (!active) return;
  if (r) {
    if (loadedRoot !== r) {
      loadedRoot = r;
      resetTree();
      void loadRoot(r);
    }
  } else {
    resetTree();
    rootError.value = "暂无工作目录";
  }
  void syncWatcher();
});

/** 仅测试用：清空模块状态 */
export function __resetSessionFsForTest() {
  active = false;
  loadedRoot = "";
  resetTree();
  resetSearchState();
  unlistenFsEvent?.();
  unlistenFsEvent = null;
  watcherStarted = false;
}
