<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import ComposerBar from "./ComposerBar.vue";
import EmptyState from "./EmptyState.vue";
import MessageItem from "./MessageItem.vue";
import { currentItems, store } from "../composables/useCodex";
import {
  buildPrefixHeights,
  computeVisibleRange,
  estimateRowHeight,
  rangeIndices,
} from "../lib/virtualList";
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

// ---------- 虚拟化：测量缓存 + 前缀和 + 可视区间 ----------
const measured = ref(new Map<string, number>());
const prefix = computed(() =>
  buildPrefixHeights(rows.value, (r) => {
    const m = measured.value.get(r.key);
    return m ?? estimateRowHeight(r);
  }),
);
const totalHeight = computed(() =>
  prefix.value.length ? prefix.value[prefix.value.length - 1] : 0,
);

const scrollTop = ref(0);
const viewport = ref(0);
const visible = computed(() =>
  computeVisibleRange(prefix.value, scrollTop.value, viewport.value, 5),
);
const visibleIndices = computed(() =>
  rangeIndices(visible.value.start, visible.value.end),
);

function topFor(i: number): number {
  return i === 0 ? 0 : prefix.value[i - 1] ?? 0;
}

// 是否吸附在底部：用户上滑查看历史时暂停自动滚动
const stickToBottom = ref(true);
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
  if (!stickToBottom.value) return;
  if (scroller.value) scroller.value.scrollTop = scroller.value.scrollHeight;
}

function onScroll() {
  const el = scroller.value;
  if (!el) return;
  scrollTop.value = el.scrollTop;
  viewport.value = el.clientHeight;
  const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
  stickToBottom.value = dist < 60;
}

function jumpToBottom() {
  stickToBottom.value = true;
  void nextTick(scrollToBottom);
}

// 行高测量：虚拟行渲染后由 ResizeObserver 回写实测高度
let rowObs: ResizeObserver | null = null;
function registerRow(el: HTMLElement | null) {
  if (!el) return;
  if (!rowObs) {
    rowObs = new ResizeObserver((entries) => {
      for (const en of entries) {
        const el2 = en.target as HTMLElement;
        const k = el2.dataset.key ?? "";
        const h = en.borderBoxSize?.[0]?.blockSize ?? en.contentRect.height;
        if (!h) continue;
        const prev = measured.value.get(k);
        if (prev == null || Math.abs(prev - h) > 0.5) {
          measured.value.set(k, h);
        }
      }
    });
  }
  rowObs.observe(el);
}

let scrollObs: ResizeObserver | null = null;

watch(
  items,
  (v, old) => {
    // 整批替换（打开历史/切换会话）时清空测量缓存
    if (v !== old) measured.value = new Map();
    void nextTick(scrollToBottom);
  },
  { deep: true },
);

// 流式内容增长导致行高变化时，保持吸底
watch(totalHeight, () => {
  if (stickToBottom.value) void nextTick(scrollToBottom);
});

onMounted(() => {
  stickToBottom.value = true;
  if (scroller.value) {
    viewport.value = scroller.value.clientHeight;
    scrollObs = new ResizeObserver(() => {
      if (scroller.value) viewport.value = scroller.value.clientHeight;
    });
    scrollObs.observe(scroller.value);
  }
  void nextTick(scrollToBottom);
});

onBeforeUnmount(() => {
  stickToBottom.value = true;
  rowObs?.disconnect();
  scrollObs?.disconnect();
  rowObs = null;
  scrollObs = null;
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
        <div
          v-else
          class="virtual-wrap"
          :style="{ height: totalHeight + 'px' }"
        >
          <div
            v-for="i in visibleIndices"
            :key="rows[i].key"
            class="vrow"
            :data-key="rows[i].key"
            :style="{ top: topFor(i) + 'px' }"
            :ref="(el) => registerRow(el as HTMLElement | null)"
          >
            <div v-if="rows[i].kind === 'sep'" class="date-sep">
              {{ rows[i].date }}
            </div>
            <MessageItem v-else :item="rows[i].item" />
          </div>
        </div>
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
  </div>
</template>
