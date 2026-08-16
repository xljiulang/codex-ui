import { computed, ref, type Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { setToast, toastError, workspace } from "./useCodex";
import { markGitStatusFresh, setGitOpInFlight } from "./useGitChanges";
import type {
  GitPullResult,
  GitPushResult,
  GitStatus,
} from "../lib/gitChanges";

/** Git 远端操作：拉取/推送/本机 git 可用性探测（忙碌互斥，成功后回写 gitStatus） */
export function useGitRemoteOps(options: { gitStatus: Ref<GitStatus | null> }) {
  /** 本机是否安装了 git（推送依赖系统 git，缺失时禁用按钮并提示） */
  const gitAvailable = ref(true);
  const pullBusy = ref(false);
  const pushBusy = ref(false);

  const branchLabel = computed(() => options.gitStatus.value?.branch ?? "");
  const repoWorkspace = computed(
    () => options.gitStatus.value?.repoWorkspace ?? "",
  );

  /** 拉取远端更新；游离 HEAD 或忙碌时禁用 */
  async function doPull() {
    if (
      pullBusy.value ||
      pushBusy.value ||
      branchLabel.value === "HEAD" ||
      !gitAvailable.value
    )
      return;
    const root = repoWorkspace.value;
    if (!root) return;
    pullBusy.value = true;
    try {
      setGitOpInFlight(true);
      const res = await invoke<GitPullResult>("git_changes_pull", {
        workspace: root,
      });
      options.gitStatus.value = res.status;
      markGitStatusFresh();
      setToast(res.message);
    } catch (e) {
      setToast(toastError(e));
    } finally {
      setGitOpInFlight(false);
      pullBusy.value = false;
    }
  }

  /** 探测本机是否安装了 git（推送可用性）；探测失败视为不可用 */
  async function checkGitAvailable() {
    try {
      gitAvailable.value = await invoke<boolean>("git_changes_git_available", {
        workspace: repoWorkspace.value || workspace.value || "",
      });
    } catch {
      gitAvailable.value = false;
    }
  }

  /** 推送当前分支到上游；游离 HEAD、未装 git 或忙碌时禁用 */
  async function doPush() {
    if (
      pushBusy.value ||
      pullBusy.value ||
      branchLabel.value === "HEAD" ||
      !gitAvailable.value
    ) {
      return;
    }
    const root = repoWorkspace.value;
    if (!root) return;
    pushBusy.value = true;
    try {
      setGitOpInFlight(true);
      const res = await invoke<GitPushResult>("git_changes_push", {
        workspace: root,
      });
      options.gitStatus.value = res.status;
      markGitStatusFresh();
      setToast(res.message);
    } catch (e) {
      setToast(toastError(e));
    } finally {
      setGitOpInFlight(false);
      pushBusy.value = false;
    }
  }

  return {
    gitAvailable,
    pullBusy,
    pushBusy,
    branchLabel,
    repoWorkspace,
    doPull,
    doPush,
    checkGitAvailable,
  };
}
