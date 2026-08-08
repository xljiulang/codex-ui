<script setup lang="ts">
import {
  deleteThread,
  openThread,
  refreshThreads,
  store,
  threadTitle,
} from "../composables/useCodex";
import { formatRelativeTime } from "../lib/format";
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
        <button class="del-btn" title="删除会话" @click.stop="deleteThread(t.id)">×</button>
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
</template>
