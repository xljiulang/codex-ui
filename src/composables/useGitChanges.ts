import { ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { setToast, toastError } from "./useCodex";
import { workspace } from "./useCodex";
import type { GitErrorCode, GitStatus } from "../lib/gitChanges";

export type GitViewState = "loading" | "ok" | "not_repo" | "error";

export const gitState = ref<GitViewState>("loading");
export const gitStatus = ref<GitStatus | null>(null);
export const gitErrorMsg = ref("");
/** 初始化进行中（防重复点击） */
export const gitInitBusy = ref(false);
/**
 * 外部（EditorPane 激活 diff 标签）请求在 Git 面板中定位某变更文件：
 * workspace=仓库根，path=文件相对/绝对路径；seq 自增保证重复定位也触发 watch。
 */
export const gitRevealTarget = ref<{
  workspace: string;
  path: string;
  seq: number;
} | null>(null);

let revealSeq = 0;

/** 请求 Git 面板定位并高亮某变更文件（与资源树定位并行，由 GitView 消费） */
export function revealGitFile(workspace: string, path: string): void {
  gitRevealTarget.value = { workspace, path, seq: ++revealSeq };
}

/** 事件冷却窗口：监听事件距上次刷新不足该时长时忽略，兜底防自激刷新 */
const REFRESH_COOLDOWN_MS = 1000;

let active = false;
let refreshing = false;
let lastRefreshAt = 0;
let unlistenGitEvent: UnlistenFn | null = null;
let watcherStarted = false;

/** 从 invoke 错误中提取 git 错误码（后端返回 { code, message }） */
function errorCodeOf(e: unknown): GitErrorCode | undefined {
  if (e && typeof e === "object") {
    const code = (e as { code?: unknown }).code;
    if (typeof code === "string") {
      return code as GitErrorCode;
    }
  }
  return undefined;
}

async function refresh() {
  if (refreshing) return;
  refreshing = true;
  lastRefreshAt = Date.now();
  try {
    const root = workspace.value;
    if (!root) {
      gitStatus.value = null;
      gitState.value = "error";
      gitErrorMsg.value = "暂无工作目录";
      return;
    }
    gitState.value = "loading";
    gitErrorMsg.value = "";
    try {
      const st = await invoke<GitStatus>("git_changes_status", { workspace: root });
      gitStatus.value = st;
      gitState.value = "ok";
    } catch (e) {
      gitStatus.value = null;
      const code = errorCodeOf(e);
      if (code === "not_a_repo") {
        gitState.value = "not_repo";
      } else {
        gitState.value = "error";
        gitErrorMsg.value = toastError(e);
      }
    }
  } finally {
    refreshing = false;
  }
}

/** 手动刷新（供 Git 更改 Tab 的刷新按钮与错误重试使用） */
export async function refreshGitChanges() {
  await refresh();
}

/** 一键初始化 Git 仓库（仅 git init，不自动提交）；成功后自动刷新状态 */
export async function initGitRepo(): Promise<boolean> {
  const root = workspace.value;
  if (!root || gitInitBusy.value) return false;
  gitInitBusy.value = true;
  try {
    await invoke("git_changes_init", { workspace: root });
    // 初始化后仓库根与 .git 才存在，重新同步监听
    await syncWatcher();
    await refresh();
    return true;
  } catch (e) {
    gitStatus.value = null;
    gitState.value = "error";
    gitErrorMsg.value = toastError(e);
    return false;
  } finally {
    gitInitBusy.value = false;
  }
}

async function syncWatcher() {
  const root = workspace.value;
  if (active && root) {
    if (!unlistenGitEvent) {
      try {
        unlistenGitEvent = await listen("git-changes/changed", () => {
          if (!active) return;
          if (Date.now() - lastRefreshAt < REFRESH_COOLDOWN_MS) return;
          void refresh();
        });
      } catch {
        // 非 Tauri 环境（如单测）忽略
      }
    }
    try {
      await invoke("git_changes_watch_start", { workspace: root });
      watcherStarted = true;
    } catch (e) {
      setToast(toastError(e));
    }
  } else {
    unlistenGitEvent?.();
    unlistenGitEvent = null;
    if (watcherStarted) {
      try {
        await invoke("git_changes_watch_stop");
      } catch {
        // 忽略停止失败
      }
      watcherStarted = false;
    }
  }
}

/** Git 更改 Tab 激活状态：激活时刷新并启动监听，切走时停止监听 */
export function setGitChangesActive(v: boolean) {
  if (active === v) return;
  active = v;
  void syncWatcher();
  if (v) void refresh();
}

// 根目录切换（切换会话/新建会话选目录）：重新检测并刷新，监听跟随新根
watch(workspace, (r, old) => {
  if (r === old) return;
  if (!active) return;
  gitStatus.value = null;
  void syncWatcher();
  void refresh();
});

/** 仅测试用：清空模块状态 */
export function __resetGitChangesForTest() {
  active = false;
  refreshing = false;
  lastRefreshAt = 0;
  gitState.value = "loading";
  gitStatus.value = null;
  gitErrorMsg.value = "";
  gitInitBusy.value = false;
  gitRevealTarget.value = null;
  revealSeq = 0;
  unlistenGitEvent?.();
  unlistenGitEvent = null;
  watcherStarted = false;
}
