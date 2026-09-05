<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import ComposerBar from "./ComposerBar.vue";
import EmptyState from "./EmptyState.vue";
import InlineInteraction from "./InlineInteraction.vue";
import MessageItem from "./MessageItem.vue";
import PlanCard from "./PlanCard.vue";
import PlanPromptBubble from "./PlanPromptBubble.vue";
import { store, type SessionTab } from "../composables/useCodex";
import { useChatScroll } from "../composables/useChatScroll";
import { useTurnNav } from "../composables/useTurnNav";
import { useWaitingChip } from "../composables/useWaitingChip";
import { ICON_ARROW_DOWN, ICON_LINES_4 } from "../lib/icons";
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
  () => props.tab.threadId,
  () => turnsBuilder.clear(),
);
const turns = computed<Turn[]>(() => turnsBuilder.build(items.value));

// 是否有正在流式输出或进行中的工具/命令（useCodex 按线程增量维护）
const hasActiveWork = computed(
  () => (store.activeWorkByThread[props.tab.threadId ?? ""] ?? 0) > 0,
);

// ---------- 等待响应提示 ----------
const { showWaiting, waitingSeconds } = useWaitingChip({
  turnActive: computed(() => props.tab.turnActive),
  hasActiveWork,
  itemCount: computed(() => items.value.length),
  interactionCount: computed(() => interactionItems.value.length),
});

// ---------- 吸底滚动引擎 + 回合导航（相互协作，接缝见各自接口） ----------
const scroll = useChatScroll(scroller, {
  isNavAnimating: () => nav.isNavAnimating(),
  onTrustedScroll: (scrollTop) => nav.onTrustedScroll(scrollTop),
  scheduleAnchorSync: () => nav.scheduleAnchorSync(),
  onSettled: () => nav.beginBackgroundWarm(),
});
// 导航浮层模板 ref（所有权在本组件模板，逻辑在 useTurnNav）
const turnNavRoot = ref<HTMLElement | null>(null);
const turnCardBody = ref<HTMLElement | null>(null);
const nav = useTurnNav({
  scroller,
  tab: computed(() => props.tab),
  items,
  stickToBottom: scroll.stickToBottom,
  isSettling: scroll.isSettling,
  turnNavRoot,
  turnCardBody,
});
// 解构保留 ref 身份：模板/编排 watch 直接使用原名（与拆分前一致）
const {
  turnNavOpen,
  hasTurnNav,
  turnEntries,
  currentIndex,
  previewItem,
  previewX,
  previewY,
  scheduleAnchorSync,
  queueBackgroundWarm,
  resetTurnNav,
  resetWarmState,
  selectTurn,
  onTurnNavEnter,
  onTurnNavLeave,
  onTurnNavKeydown,
  onTurnItemEnter,
  onTurnItemLeave,
  onTurnPreviewEnter,
  attachResizeObserver,
  dispose: disposeNav,
} = nav;
const {
  stickToBottom,
  onScroll,
  scheduleScroll,
  jumpToBottom,
  settleToBottom,
  resetForNewThread,
  attachObserver,
  dispose: disposeScroll,
} = scroll;

// ---------- 无障碍播报（回合开始/结束） ----------
const liveAnnouncement = ref("");
watch(
  () => props.tab.turnActive,
  (v, old) => {
    if (v && !old) liveAnnouncement.value = "正在生成回复";
    else if (!v && old) liveAnnouncement.value = "回复完成";
  },
);

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
    scheduleAnchorSync();
  },
);

// 新会话消息出现/内容由空转非空时补后台预热（历史会话由 loading/thread watcher 触发）
watch(
  () => items.value.length,
  () => {
    if (items.value.length > 0) queueBackgroundWarm();
  },
);

// 切换会话（新建/打开历史/会话失效重置）后重置吸底与导航状态，
// 避免旧会话的上滑状态残留导致新对话默认不吸底
watch(
  () => props.tab.threadId,
  () => {
    resetTurnNav();
    // 切回已加载会话：内容可能仍处 content-visibility 估算布局，直接吸底会停在最新消息之上
    if (items.value.length > 0) {
      void settleToBottom();
    } else {
      resetForNewThread();
    }
  },
);

// 历史会话加载完成（loading true→false）：消息整批落地后强制测量并滚到最新
watch(
  () => props.tab.loading,
  (v, old) => {
    if (old && !v) {
      resetWarmState();
      if (items.value.length > 0) {
        void settleToBottom();
      }
    }
  },
);

onMounted(() => {
  attachObserver();
  attachResizeObserver();
});

onBeforeUnmount(() => {
  disposeScroll();
  disposeNav();
});
</script>

<template>
  <div class="chat">
    <div class="chat-scroll-wrap">
      <div ref="scroller" class="chat-scroll" @scroll="onScroll">
        <div v-if="items.length === 0" class="chat-empty">
          <EmptyState :busy="tab.turnActive || tab.creatingSession" />
        </div>
        <template v-for="turn in turns" :key="turn.key">
          <section class="turn">
            <template v-for="row in turn.rows" :key="row.key">
              <div v-if="row.kind === 'sep'" class="date-sep">{{ row.date }}</div>
              <MessageItem v-else :item="row.item" :tab="props.tab" />
            </template>
          </section>
        </template>
        <PlanCard v-if="tab.plan" :plan="tab.plan" />
        <InlineInteraction :interactions="interactionItems" />
        <PlanPromptBubble :prompt="tab.planPrompt" />
        <div v-if="showWaiting" class="waiting-chip">
          等待响应({{ waitingSeconds }}s)
          <span class="dot"></span>
          <span class="dot"></span>
          <span class="dot"></span>
        </div>
        <div v-if="tab.loading" class="loading-thread-note">加载会话…</div>
      </div>
      <div
        v-if="hasTurnNav"
        ref="turnNavRoot"
        class="turn-nav"
        @mouseenter="onTurnNavEnter()"
        @mouseleave="onTurnNavLeave()"
      >
        <button
          type="button"
          class="turn-nav-btn"
          :aria-expanded="turnNavOpen ? 'true' : 'false'"
          aria-label="回合导航"
          @keydown="onTurnNavKeydown"
        >
          <svg viewBox="0 0 24 24" preserveAspectRatio="none" aria-hidden="true">
            <path :d="ICON_LINES_4" />
          </svg>
        </button>
        <div v-if="turnNavOpen" class="turn-nav-card">
          <div ref="turnCardBody" class="turn-nav-list">
            <template v-if="turnEntries.length">
              <button
                v-for="entry in turnEntries"
                :key="entry.id"
                type="button"
                class="turn-nav-item"
                :class="{ active: entry.index === currentIndex }"
                @click="selectTurn(entry.index)"
                @mouseenter="onTurnItemEnter($event, entry)"
                @mouseleave="onTurnItemLeave()"
              >
                <span class="turn-nav-item-index">{{ entry.index + 1 }}</span>
                <span class="turn-nav-item-title">
                  {{ entry.title || "（无文字内容）" }}
                </span>
                <span v-if="entry.time" class="turn-nav-item-time">
                  {{ entry.time }}
                </span>
              </button>
            </template>
            <div v-else class="turn-nav-empty">暂无回合</div>
          </div>
        </div>
        <div
          v-if="previewItem && turnNavOpen"
          class="turn-nav-preview"
          :style="{ left: previewX + 'px', top: previewY + 'px' }"
          @mouseenter="onTurnPreviewEnter()"
        >
          <MessageItem :item="previewItem" :tab="props.tab" />
        </div>
      </div>
      <button
        v-if="!stickToBottom"
        type="button"
        class="scroll-bottom-btn"
        aria-label="回到底部"
        @click="jumpToBottom()"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path :d="ICON_ARROW_DOWN" />
        </svg>
      </button>
    </div>
    <ComposerBar :tab="tab" :active="active" />
    <div class="sr-only" aria-live="polite">{{ liveAnnouncement }}</div>
  </div>
</template>
