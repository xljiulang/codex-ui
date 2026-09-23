<script setup lang="ts">
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { setToast, toastError } from "../composables/useCodex";
import { setGitOpInFlight } from "../composables/useGitChanges";
import type { GitStatus } from "../lib/gitChanges";
import { ICON_CHECK } from "../lib/icons";

const props = defineProps<{
  /** 仓库根目录（git_changes_commit 的 workspace 参数） */
  workspace: string;
  /** 已暂存文件数（提交可用性与提示文案） */
  stagedCount: number;
}>();

const emit = defineEmits<{
  /** 提交成功：回传最新状态（父组件刷新 gitStatus 与提交历史） */
  committed: [status: GitStatus];
}>();

const commitMessage = ref("");
const commitBusy = ref(false);

const canCommit = computed(
  () =>
    !commitBusy.value && !!commitMessage.value.trim() && props.stagedCount > 0,
);

const commitHint = computed(() =>
  props.stagedCount > 0
    ? `将提交 ${props.stagedCount} 个文件`
    : "先在上方暂存更改",
);

/** 提交已暂存更改；成功后清空消息并刷新状态 */
async function doCommit() {
  if (!canCommit.value) return;
  if (!props.workspace) return;
  commitBusy.value = true;
  try {
    setGitOpInFlight(true);
    const st = await invoke<GitStatus>("git_changes_commit", {
      workspace: props.workspace,
      message: commitMessage.value.trim(),
    });
    commitMessage.value = "";
    setToast("提交成功");
    emit("committed", st);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    setGitOpInFlight(false);
    commitBusy.value = false;
  }
}
</script>

<template>
  <div class="git-commit-bar">
    <textarea
      v-model="commitMessage"
      class="git-commit-input"
      rows="2"
      placeholder="提交消息（Ctrl+Enter 提交）"
      :disabled="commitBusy"
      @keydown.ctrl.enter="doCommit()"
    ></textarea>
    <div class="git-commit-row">
      <button
        class="btn sm git-commit-btn"
        :disabled="!canCommit"
        @click="doCommit()"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path :d="ICON_CHECK" />
        </svg>
        <span>{{ commitBusy ? "提交中…" : "提交" }}</span>
      </button>
      <span class="git-commit-hint">{{ commitHint }}</span>
    </div>
  </div>
</template>
