<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import ComposerBar from "./ComposerBar.vue";
import EmptyState from "./EmptyState.vue";
import MessageItem from "./MessageItem.vue";
import { currentItems, store } from "../composables/useCodex";
import type { ThreadItem } from "../lib/types";

const scroller = ref<HTMLElement | null>(null);
const items = computed(() => currentItems());

type Row =
  | { key: string; kind: "sep"; date: string }
  | { key: string; kind: "msg"; item: ThreadItem };

function formatDay(ts: number): string {
  const d = new Date(ts);
  const now = new Date();
  return d.getFullYear() === now.getFullYear()
    ? `${d.getMonth() + 1}月${d.getDate()}日`
    : `${d.getFullYear()}年${d.getMonth() + 1}月${d.getDate()}日`;
}

// 跨天时插入日期分隔线（时间戳缺失的消息不产生分隔）；每行带稳定 key
const rows = computed<Row[]>(() => {
  const out: Row[] = [];
  let lastDay = "";
  let sepCount = 0;
  for (const item of items.value) {
    const ts = item.startedAtMs as number | undefined;
    if (typeof ts === "number") {
      const day = formatDay(ts);
      if (day !== lastDay) {
        out.push({ key: `sep-${day}-${sepCount++}`, kind: "sep", date: day });
        lastDay = day;
      }
    }
    out.push({ key: item.id, kind: "msg", item });
  }
  return out;
});

// 是否吸附在底部：用户上滑查看历史时暂停自动滚动
const stickToBottom = ref(true);
// 是否有正在流式输出或进行中的工具/命令（useCodex 按线程增量维护）
const hasActiveWork = computed(
  () => (store.activeWorkByThread[store.currentThreadId ?? ""] ?? 0) > 0,
);

// ---------- 无障碍播报（回合开始/结束） ----------
const liveAnnouncement = ref("");
watch(
  () => store.turnActive,
  (v, old) => {
    if (v && !old) liveAnnouncement.value = "正在生成回复";
    else if (!v && old) liveAnnouncement.value = "回复完成";
  },
);

function scrollToBottom() {
  if (!stickToBottom.value) return;
  if (scroller.value) scroller.value.scrollTop = scroller.value.scrollHeight;
}

function onScroll() {
  const el = scroller.value;
  if (!el) return;
  const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
  stickToBottom.value = dist < 60;
}

function jumpToBottom() {
  stickToBottom.value = true;
  void nextTick(scrollToBottom);
}

watch(
  items,
  () => {
    void nextTick(scrollToBottom);
  },
  { deep: true },
);

onMounted(() => {
  stickToBottom.value = true;
  void nextTick(scrollToBottom);
});

onBeforeUnmount(() => {
  // 避免切换视图后残留滚动状态
  stickToBottom.value = true;
});
</script>

<template>
  <div class="chat">
    <div v-if="store.turnActive" class="thinking-bar"></div>
    <div class="chat-scroll-wrap">
      <div ref="scroller" class="chat-scroll" @scroll="onScroll">
        <div v-if="items.length === 0" class="chat-empty">
          <EmptyState :busy="store.turnActive || store.busy" />
        </div>
        <template v-for="row in rows" :key="row.key">
          <div v-if="row.kind === 'sep'" class="date-sep">{{ row.date }}</div>
          <MessageItem v-else :item="row.item" />
        </template>
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
      <div v-if="!stickToBottom" class="scroll-bottom-btn" @click="jumpToBottom()">
        ↓ 回到底部
      </div>
    </div>
    <ComposerBar />
    <div class="sr-only" aria-live="polite">{{ liveAnnouncement }}</div>
  </div>
</template>
