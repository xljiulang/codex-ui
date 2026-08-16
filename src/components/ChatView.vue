<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import ComposerBar from "./ComposerBar.vue";
import EmptyState from "./EmptyState.vue";
import InlineInteraction from "./InlineInteraction.vue";
import MessageItem from "./MessageItem.vue";
import PlanPromptBubble from "./PlanPromptBubble.vue";
import { store, type SessionTab } from "../composables/useCodex";
import { createTurnsBuilder, type Turn } from "../lib/turns";

const scroller = ref<HTMLElement | null>(null);
const props = defineProps<{ tab: SessionTab; active?: boolean }>();
const items = computed(() =>
  props.tab.threadId ? (store.itemsByThread[props.tab.threadId] ?? []) : [],
);
/** 该标签的待处理交互：绑定线程的走标签记录，未绑定（新对话）回退全局 */
const interactionItems = computed(() =>
  props.tab.threadId ? (props.tab.interactions ?? []) : store.interactions,
);

function formatDay(ts: number): string {
  const d = new Date(ts);
  const now = new Date();
  return d.getFullYear() === now.getFullYear()
    ? `${d.getMonth() + 1}月${d.getDate()}日`
    : `${d.getFullYear()}年${d.getMonth() + 1}月${d.getDate()}日`;
}

// 按回合分组：userMessage 起始新回合，行对象按 key 缓存复用
const turnsBuilder = createTurnsBuilder(formatDay);
watch(
  () => store.currentThreadId,
  () => turnsBuilder.clear(),
);
const turns = computed<Turn[]>(() => turnsBuilder.build(items.value));

// 是否吸附在底部：用户上滑查看历史时暂停自动滚动
const stickToBottom = ref(true);
// 最近一次程序化吸底写入/滚动事件后的 scrollTop：
// 用户可信滚动相对该位置向上移动即视为“上滑看历史”，一次正常上滑立即解除；
// 向下（追赶窗口内）或程序化滚动不会误解除。
let lastStickScrollTop = 0;
// 解除吸底所需的最小距底距离：轻微误触/抖动（< 32px）不解除，
// 单次正常滚轮（约 80-100px）一次即解除
const UNPIN_DIST_PX = 32;
// 是否有正在流式输出或进行中的工具/命令（useCodex 按线程增量维护）
const hasActiveWork = computed(
  () => (store.activeWorkByThread[props.tab.threadId ?? ""] ?? 0) > 0,
);

// ---------- 无障碍播报（回合开始/结束） ----------
const liveAnnouncement = ref("");
watch(
  () => props.tab.turnActive,
  (v, old) => {
    if (v && !old) liveAnnouncement.value = "正在生成回复";
    else if (!v && old) liveAnnouncement.value = "回复完成";
  },
);

function onScroll(e: Event) {
  const el = scroller.value;
  if (!el) return;
  const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
  if (e.isTrusted && dist > UNPIN_DIST_PX && el.scrollTop < lastStickScrollTop - 2) {
    // 用户向上滚动且距底部超过阈值才解除：
    // 轻微误触/抖动（< 32px）不会在流式期间误解除，单次正常滚轮一次即生效；
    // 程序化吸底写入与追赶窗口内的向下滚动不会误判
    stickToBottom.value = false;
  } else if (dist < 60) {
    stickToBottom.value = true;
  }
  lastStickScrollTop = el.scrollTop;
}

// 吸底滚动合并到每帧一次：流式高频变更时避免每次都强制整块布局
let scrollRaf: number | undefined;
function scheduleScroll() {
  if (!stickToBottom.value || scrollRaf !== undefined) return;
  scrollRaf = requestAnimationFrame(() => {
    scrollRaf = undefined;
    if (!stickToBottom.value) return;
    if (scroller.value) {
      scroller.value.scrollTop = scroller.value.scrollHeight;
      lastStickScrollTop = scroller.value.scrollTop;
    }
    // 次帧再跟一次：content-visibility 解除跳过渲染后 scrollHeight 才更新，
    // 避免吸底落在估算高度之上
    requestAnimationFrame(() => {
      if (stickToBottom.value && scroller.value) {
        scroller.value.scrollTop = scroller.value.scrollHeight;
        lastStickScrollTop = scroller.value.scrollTop;
      }
    });
  });
}

function jumpToBottom() {
  stickToBottom.value = true;
  scheduleScroll();
}

/**
 * 历史会话整批加载/切回后强制滚到最新：`.msg` 使用 content-visibility（未渲染时按
 * 80px 估算高度），直接 scrollTop=scrollHeight 会落在估算底部、高于真实底部，且
 * content-visibility 的布局变化不触发 MutationObserver。给 scroller 临时加 measuring
 * 类一次性真实渲染全部消息（contain-intrinsic-size: auto 会记住真实高度），
 * 测量后滚到底部再移除类，随后复用双 rAF 吸底兜底。
 */
async function settleToBottom() {
  const el = scroller.value;
  if (!el) return;
  stickToBottom.value = true;
  el.classList.add("measuring");
  try {
    await nextTick();
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    el.scrollTop = el.scrollHeight;
    lastStickScrollTop = el.scrollTop;
  } finally {
    el.classList.remove("measuring");
  }
  scheduleScroll();
}

// 用户手动发送消息后强制恢复吸底：即使此前上滑查看历史已解除吸底，
// 发送动作也应立即回到底部并继续跟随；排队消息自动发送不触发（不打断阅读位置）
watch(
  () => store.userSendRev,
  () => {
    jumpToBottom();
  },
);

watch(
  [
    () => items.value.length,
    () => store.itemsRev,
    () => interactionItems.value.length,
    () => props.tab.planPrompt,
  ],
  () => {
    scheduleScroll();
  },
);

// 切换会话（新建/打开历史/会话失效重置）后重置吸底状态，
// 避免旧会话的上滑状态残留导致新对话默认不吸底
watch(
  () => store.currentThreadId,
  () => {
    // 切回已加载会话：内容可能仍处 content-visibility 估算布局，直接吸底会停在最新消息之上
    if (items.value.length > 0) {
      void settleToBottom();
    } else {
      stickToBottom.value = true;
      lastStickScrollTop = 0;
      scheduleScroll();
    }
  },
);

// 历史会话加载完成（loading true→false）：消息整批落地后强制测量并滚到最新
watch(
  () => props.tab.loading,
  (v, old) => {
    if (old && !v && items.value.length > 0) {
      void settleToBottom();
    }
  },
);

// 懒加载/异步图片加载后高度变化没有 DOM 结构变更，补一次跟随
function onImageLoad(e: Event) {
  if ((e.target as Element | null)?.tagName === "IMG") scheduleScroll();
}

// DOM 高度兜底：worker 渲染 markdown / 命令输出 HTML 落地晚于数据变更，
// 结构变化时在微任务内立即吸底（不等下一帧），再以 rAF 合并高频变更
let scrollObserver: MutationObserver | undefined;
onMounted(() => {
  stickToBottom.value = true;
  if (scroller.value && typeof MutationObserver !== "undefined") {
    scrollObserver = new MutationObserver(() => {
      if (stickToBottom.value && scroller.value) {
        scroller.value.scrollTop = scroller.value.scrollHeight;
        lastStickScrollTop = scroller.value.scrollTop;
      }
      scheduleScroll();
    });
    scrollObserver.observe(scroller.value, { childList: true, subtree: true });
    scroller.value.addEventListener("load", onImageLoad, true);
    lastStickScrollTop = scroller.value.scrollTop;
  }
  scheduleScroll();
});

onBeforeUnmount(() => {
  if (scrollRaf !== undefined) cancelAnimationFrame(scrollRaf);
  scrollRaf = undefined;
  scrollObserver?.disconnect();
  scrollObserver = undefined;
  scroller.value?.removeEventListener("load", onImageLoad, true);
  // 避免切换视图后残留滚动状态
  stickToBottom.value = true;
});
</script>

<template>
  <div class="chat">
    <div v-if="tab.turnActive" class="thinking-bar"></div>
    <div class="chat-scroll-wrap">
      <div ref="scroller" class="chat-scroll" @scroll="onScroll">
        <div v-if="items.length === 0" class="chat-empty">
          <EmptyState :busy="tab.turnActive || store.busy" />
        </div>
        <template v-for="turn in turns" :key="turn.key">
          <section class="turn">
            <template v-for="row in turn.rows" :key="row.key">
              <div v-if="row.kind === 'sep'" class="date-sep">{{ row.date }}</div>
              <MessageItem v-else :item="row.item" />
            </template>
          </section>
        </template>
        <InlineInteraction :interactions="interactionItems" />
        <PlanPromptBubble :prompt="tab.planPrompt" />
        <div
          v-if="
            tab.turnActive &&
            !hasActiveWork &&
            items.length &&
            interactionItems.length === 0
          "
          class="thinking-chip"
        >
          思考中
          <span class="dot"></span>
          <span class="dot"></span>
          <span class="dot"></span>
        </div>
        <div v-if="tab.loading" class="loading-thread-note">加载会话…</div>
      </div>
      <div v-if="!stickToBottom" class="scroll-bottom-btn" @click="jumpToBottom()">
        ↓ 回到底部
      </div>
    </div>
    <ComposerBar :tab="tab" :active="active" />
    <div class="sr-only" aria-live="polite">{{ liveAnnouncement }}</div>
  </div>
</template>
