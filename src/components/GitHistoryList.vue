<script setup lang="ts">
import { ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { openCommitTab } from "../composables/useEditorTabs";
import type { GitCommitEntry, GitStatus } from "../lib/gitChanges";
import { formatDateTime } from "../lib/format";
import { ICON_MORE } from "../lib/icons";

const props = defineProps<{
  /** 仓库根目录（git_changes_log 的 workspace 参数） */
  workspace: string;
  /** gitStatus 引用变化（刷新/提交/合并/切分支）时重载第一页 */
  reloadKey: GitStatus | null;
}>();

/** 提交历史（最新在前），git_changes_log 返回 */
const commits = ref<GitCommitEntry[]>([]);
const logBusy = ref(false);
/** 是否还有更旧的提交可加载（上一批返回满 50 条即视为还有更多） */
const logHasMore = ref(false);
const LOG_LIMIT = 50;

/** 重载提交历史第一页（最新在前）；由 reloadKey 变化/手动刷新触发 */
async function loadCommitLog() {
  const root = props.workspace;
  if (!root || logBusy.value) return;
  logBusy.value = true;
  try {
    const res = await invoke<GitCommitEntry[]>("git_changes_log", {
      workspace: root,
      limit: LOG_LIMIT,
      before: null,
    });
    commits.value = Array.isArray(res) ? res : [];
    logHasMore.value = commits.value.length === LOG_LIMIT;
  } catch {
    commits.value = [];
    logHasMore.value = false;
  } finally {
    logBusy.value = false;
  }
}

/** 加载更多提交历史：以当前最后一条 hash 为游标续页并追加 */
async function loadMoreCommits() {
  const root = props.workspace;
  const last = commits.value[commits.value.length - 1];
  if (!root || logBusy.value || !last) return;
  logBusy.value = true;
  try {
    const res = await invoke<GitCommitEntry[]>("git_changes_log", {
      workspace: root,
      limit: LOG_LIMIT,
      before: last.hash,
    });
    if (!Array.isArray(res) || !res.length) {
      // 已到历史尽头（含游标失效/重写场景）：隐藏按钮，保留已加载列表
      logHasMore.value = false;
      return;
    }
    commits.value = commits.value.concat(res);
    logHasMore.value = res.length === LOG_LIMIT;
  } catch {
    // 加载更多失败保持现状，按钮保留以便重试
  } finally {
    logBusy.value = false;
  }
}

/** 单击提交记录：打开提交详情标签（同一提交重复点击仅激活已有标签） */
function openCommit(commit: GitCommitEntry) {
  void openCommitTab(props.workspace, commit.hash, commit.subject);
}

// gitStatus 每次刷新（含提交/合并/拉取/切分支）后同步刷新提交历史
watch(
  () => props.reloadKey,
  () => void loadCommitLog(),
  { immediate: true },
);
</script>

<template>
  <div v-if="commits.length" class="git-log-list">
    <button
      v-for="c in commits"
      :key="c.hash"
      class="git-log-item"
      :aria-label="'查看提交 ' + c.subject"
      @click="openCommit(c)"
    >
      <span class="git-log-dot" aria-hidden="true"></span>
      <div class="git-log-main">
        <span class="git-log-subject">{{ c.subject }}</span>
        <span class="git-log-meta">
          {{ c.author }} · {{ formatDateTime(c.timeSecs) }}
        </span>
      </div>
      <span class="git-log-hash" v-tooltip="c.hash">{{ c.shortHash }}</span>
    </button>
    <button
      v-if="logHasMore"
      class="git-icon-btn git-log-more"
      aria-label="加载更多"
      v-tooltip="'加载更多'"
      :disabled="logBusy"
      @click="loadMoreCommits()"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path :d="ICON_MORE" />
      </svg>
    </button>
  </div>
  <div v-else-if="logBusy" class="git-section-empty">加载中…</div>
  <div v-else class="git-section-empty">暂无提交记录</div>
</template>
