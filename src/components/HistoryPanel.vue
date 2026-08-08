<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import {
  deleteThread,
  openThread,
  refreshThreads,
  store,
  threadTitle,
} from "../composables/useCodex";
import { formatRelativeTime } from "../lib/format";
import type { ThreadSummary } from "../lib/types";

const confirmThread = ref<ThreadSummary | null>(null);
const confirmEl = ref<HTMLElement | null>(null);
let lastFocus: HTMLElement | null = null;

function askDelete(t: ThreadSummary) {
  confirmThread.value = t;
  lastFocus = document.activeElement as HTMLElement | null;
  void nextTick(() => {
    confirmEl.value
      ?.querySelector<HTMLElement>(".modal-foot .btn.danger")
      ?.focus();
  });
}

function cancelDelete() {
  confirmThread.value = null;
  lastFocus?.focus?.();
  lastFocus = null;
}

async function doDelete() {
  const t = confirmThread.value;
  if (!t) return;
  confirmThread.value = null;
  lastFocus?.focus?.();
  lastFocus = null;
  await deleteThread(t.id);
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape" && confirmThread.value) cancelDelete();
}

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
  // 每次打开面板都重新拉取，避免显示已被删除/新增的过期数据
  void refreshThreads();
});
onBeforeUnmount(() => window.removeEventListener("keydown", onKeydown));
</script>

<template>
  <aside class="history-panel">
    <div class="history-head">历史记录</div>
    <div class="history-list">
      <div
        v-for="t in store.threads"
        :key="t.id"
        class="history-item"
        :class="{ active: t.id === store.currentThreadId }"
        @click="openThread(t.id)"
      >
        <span class="history-main">
          <span class="history-title">{{ threadTitle(t) }}</span>
          <span v-if="t.cwd" class="history-cwd" :title="t.cwd">{{ t.cwd }}</span>
        </span>
        <span class="history-time">{{ formatRelativeTime(t.recencyAt ?? t.updatedAt) }}</span>
        <button class="del-btn" title="删除会话" @click.stop="askDelete(t)">×</button>
      </div>
      <div v-if="!store.threads.length && !store.loadingHistory" class="menu-note">
        暂无会话
      </div>
      <div v-if="store.loadingHistory && !store.threads.length" class="menu-note">
        加载中…
      </div>
    </div>
    <button
      v-if="store.nextCursor"
      class="history-more"
      :disabled="store.loadingHistory"
      @click="refreshThreads(true)"
    >
      查看全部（{{ store.threadsTotal }} 个）
    </button>
  </aside>

  <div v-if="confirmThread" class="modal-mask">
    <div ref="confirmEl" class="modal" tabindex="-1">
      <div class="modal-head">
        <span class="modal-title">删除会话</span>
      </div>
      <div class="modal-body">
        确定删除会话「{{ threadTitle(confirmThread) }}」吗？此操作不可恢复。
      </div>
      <div class="modal-foot">
        <button class="btn" @click="cancelDelete()">取消</button>
        <button class="btn danger" @click="doDelete()">删除</button>
      </div>
    </div>
  </div>
</template>
