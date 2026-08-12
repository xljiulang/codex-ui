<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
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

function onScroll() {
  const el = scroller.value;
  if (!el) return;
  const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
  stickToBottom.value = dist < 60;
}

/** 最后一条消息的变更指纹：只跟踪末尾消息的流式/状态与内容增长，历史消息变更零开销 */
function contentSize(it: ThreadItem): number {
  let n = 0;
  const add = (v: unknown) => {
    if (typeof v === "string") n += v.length;
    else if (typeof v === "number") n += v;
    else if (Array.isArray(v)) for (const x of v) add(x);
  };
  // 覆盖普通回复 text、命令输出 aggregatedOutput/output、思考过程 content/summary
  add(it.text);
  add(it.aggregatedOutput);
  add(it.output);
  add(it.content);
  add(it.summary);
  return n;
}

const lastItemKey = computed(() => {
  const it = items.value[items.value.length - 1];
  if (!it) return "";
  return [
    it.id,
    it.streaming ? 1 : 0,
    String(it.status ?? ""),
    contentSize(it),
  ].join(":");
});

// 吸底滚动合并到每帧一次：流式高频变更时避免每次都强制整块布局
let scrollRaf: number | undefined;
function scheduleScroll() {
  if (!stickToBottom.value || scrollRaf !== undefined) return;
  scrollRaf = requestAnimationFrame(() => {
    scrollRaf = undefined;
    if (!stickToBottom.value) return;
    if (scroller.value) scroller.value.scrollTop = scroller.value.scrollHeight;
  });
}

function jumpToBottom() {
  stickToBottom.value = true;
  scheduleScroll();
}

watch(
  [() => items.value.length, lastItemKey],
  () => {
    scheduleScroll();
  },
);

onMounted(() => {
  stickToBottom.value = true;
  scheduleScroll();
});

onBeforeUnmount(() => {
  if (scrollRaf !== undefined) cancelAnimationFrame(scrollRaf);
  scrollRaf = undefined;
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
