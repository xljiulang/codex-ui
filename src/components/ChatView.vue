<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import ComposerBar from "./ComposerBar.vue";
import EmptyState from "./EmptyState.vue";
import MessageItem from "./MessageItem.vue";
import { currentItems, store } from "../composables/useCodex";

const scroller = ref<HTMLElement | null>(null);
const items = computed(() => currentItems());
// 是否有正在流式输出或进行中的工具/命令
const hasActiveWork = computed(() =>
  items.value.some(
    (i) =>
      i.streaming === true ||
      ["in_progress", "inProgress", "pending", "started"].includes(
        String(i.status ?? ""),
      ),
  ),
);

function scrollToBottom() {
  if (scroller.value) scroller.value.scrollTop = scroller.value.scrollHeight;
}

watch(
  items,
  () => {
    void nextTick(scrollToBottom);
  },
  { deep: true },
);
</script>

<template>
  <div class="chat">
    <div v-if="store.turnActive" class="thinking-bar"></div>
    <div ref="scroller" class="chat-scroll">
      <div v-if="items.length === 0" class="chat-empty">
        <EmptyState :busy="store.turnActive || store.busy" />
      </div>
      <MessageItem v-for="item in items" :key="item.id" :item="item" />
      <div
        v-if="store.turnActive && !hasActiveWork && items.length"
        class="thinking-chip"
      >
        思考中
        <span class="dot"></span>
        <span class="dot"></span>
        <span class="dot"></span>
      </div>
      <div v-if="store.loadingThread" class="loading-thread-note">加载会话…</div>
      <div v-if="store.turnInterrupted" class="stop-note">已停止生成</div>
    </div>
    <ComposerBar />
  </div>
</template>
