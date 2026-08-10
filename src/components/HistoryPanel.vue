<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import {
  clearSearch,
  deleteThread,
  openThread,
  renameThread,
  refreshThreads,
  searchThreads,
  store,
  threadTitle,
  togglePin,
} from "../composables/useCodex";
import { formatRelativeTime } from "../lib/format";
import type { ThreadSummary } from "../lib/types";

const confirmThread = ref<ThreadSummary | null>(null);
const confirmEl = ref<HTMLElement | null>(null);
const searchTerm = ref("");
const editingId = ref<string | null>(null);
const editName = ref("");
let searchTimer: number | undefined;
let lastFocus: HTMLElement | null = null;

function onSearchInput() {
  if (searchTimer) window.clearTimeout(searchTimer);
  searchTimer = window.setTimeout(() => {
    void searchThreads(searchTerm.value);
  }, 300);
}

function clearSearchInput() {
  searchTerm.value = "";
  if (searchTimer) window.clearTimeout(searchTimer);
  clearSearch();
}

function startRename(t: ThreadSummary) {
  editingId.value = t.id;
  editName.value = threadTitle(t);
  void nextTick(() => {
    (document.querySelector<HTMLInputElement>(".rename-input"))?.focus();
  });
}

function saveRename(t: ThreadSummary) {
  if (editingId.value !== t.id) return;
  editingId.value = null;
  if (editName.value.trim() && editName.value.trim() !== threadTitle(t)) {
    void renameThread(t.id, editName.value.trim());
  }
}

function cancelRename() {
  editingId.value = null;
}

function loadMore() {
  if (store.searchActive) {
    void searchThreads(searchTerm.value, true);
  } else {
    void refreshThreads(true);
  }
}

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
    <div class="history-head">
      <input
        v-model="searchTerm"
        class="history-search"
        type="text"
        placeholder="搜索历史…"
        @input="onSearchInput()"
      />
      <button
        v-if="searchTerm"
        class="history-search-clear"
        aria-label="清除搜索"
        v-tooltip="'清除搜索'"
        @click="clearSearchInput()"
      >
        ×
      </button>
    </div>
    <div class="history-list">
      <div
        v-for="t in store.threads"
        :key="t.id"
        class="history-item"
        :class="{ active: t.id === store.currentThreadId }"
        @click="openThread(t.id)"
      >
        <span class="history-main">
          <input
            v-if="editingId === t.id"
            v-model="editName"
            class="rename-input"
            @click.stop
            @keydown.enter="saveRename(t)"
            @keydown.esc="cancelRename()"
            @blur="saveRename(t)"
          />
          <template v-else>
            <span class="history-title-row">
              <span class="history-title">{{ threadTitle(t) }}</span>
              <span v-if="t.isPinned" class="pin-badge">置顶</span>
            </span>
            <span
              v-if="store.searchActive && store.searchSnippets[t.id]"
              class="history-snippet"
            >
              {{ store.searchSnippets[t.id] }}
            </span>
            <span v-if="t.cwd" class="history-cwd" v-tooltip="t.cwd">{{ t.cwd }}</span>
          </template>
        </span>
        <span class="history-time">{{ formatRelativeTime(t.recencyAt ?? t.updatedAt) }}</span>
        <span class="history-actions">
          <button
            class="act-btn"
            :class="{ pinned: t.isPinned }"
            :aria-label="t.isPinned ? '取消固定' : '固定置顶'"
            v-tooltip="t.isPinned ? '取消固定' : '固定置顶'"
            @click.stop="togglePin(t.id, !t.isPinned)"
          >
            <svg viewBox="0 0 24 24">
              <path
                d="M14 4h-4l1 7-3 2v2h8v-2l-3-2z"
                :fill="t.isPinned ? 'currentColor' : 'none'"
              />
            </svg>
          </button>
          <button
            class="act-btn"
            aria-label="重命名"
            v-tooltip="'重命名'"
            @click.stop="startRename(t)"
          >
            <svg viewBox="0 0 24 24">
              <path
                d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"
              />
            </svg>
          </button>
          <button
            class="act-btn del"
            aria-label="删除会话"
            v-tooltip="'删除会话'"
            @click.stop="askDelete(t)"
          >
            ×
          </button>
        </span>
      </div>
      <div v-if="!store.threads.length && !store.loadingHistory" class="menu-note">
        暂无会话
      </div>
      <div v-if="store.loadingHistory && !store.threads.length" class="menu-note">
        加载中…
      </div>
    </div>
    <button
      v-if="store.searchActive ? store.searchCursor : store.nextCursor"
      class="history-more"
      :disabled="store.loadingHistory"
      @click="loadMore()"
    >
      加载更多
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
