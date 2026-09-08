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

<style scoped>
/* 聊天区 */
.chat {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  min-height: 0;
  position: relative;
}

/* “等待响应”发光提示 */
.waiting-chip {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-5);
  border-radius: 999px;
  font-size: var(--font-sm);
  font-weight: 600;
  /* 计时数字等宽，逐 100ms 刷新时宽度不抖动 */
  font-variant-numeric: tabular-nums;
  color: var(--accent);
  background: rgba(var(--accent-rgb), 0.08);
  border: 1px solid rgba(var(--accent-rgb), 0.32);
  box-shadow: 0 0 16px rgba(var(--accent-rgb), 0.14);
  margin-top: var(--space-4);
}

.waiting-chip .dot {
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: var(--accent);
  animation: dot-blink 1.2s ease-in-out infinite;
}

.waiting-chip .dot:nth-child(2) {
  animation-delay: 0.2s;
}

.waiting-chip .dot:nth-child(3) {
  animation-delay: 0.4s;
}

.chat-scroll-wrap {
  position: relative;
  flex: 1;
  min-height: 0;
  display: flex;
}

.chat-scroll {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-10) var(--space-12) var(--space-5);
}

.scroll-bottom-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  position: absolute;
  left: 50%;
  bottom: 10px;
  transform: translateX(-50%);
  z-index: 20;
  width: 30px;
  height: 30px;
  padding: 0;
  border-radius: 50%;
  color: var(--text-bright);
  background: var(--bg-panel);
  border: 1px solid var(--border-light);
  box-shadow: var(--shadow-md);
  cursor: pointer;
  opacity: 0.9;
  transition: opacity var(--ease), border-color var(--ease);
}

.scroll-bottom-btn svg {
  width: 16px;
  height: 16px;
  fill: currentColor;
}

.scroll-bottom-btn:hover {
  opacity: 1;
  border-color: var(--accent-dim);
}

/* 回合定位：与滚动条同宽、完全贴右的全高锚点条，按钮垂直居中；卡片悬停后从按钮左侧全高弹出 */
.turn-nav {
  position: absolute;
  top: 0;
  bottom: 0;
  right: 0;
  width: 12px;
  z-index: 30;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  pointer-events: none;
}

.turn-nav-btn {
  pointer-events: auto;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 10px;
  height: 30px;
  padding: 0;
  border: none;
  border-radius: 6px;
  color: var(--text-faint);
  background: none;
  box-shadow: none;
  cursor: pointer;
  opacity: 0.8;
  transition: opacity var(--ease), color var(--ease);
}

.turn-nav-btn svg {
  width: 100%;
  height: 100%;
  fill: currentColor;
}

.turn-nav-btn:hover:not(:disabled) {
  opacity: 1;
  color: var(--text-dim);
}

.turn-nav-btn:disabled {
  opacity: 0.35;
  cursor: default;
}

.turn-nav-card {
  pointer-events: auto;
  position: absolute;
  top: 0;
  bottom: 0;
  right: 100%;
  z-index: 30;
  width: 280px;
  max-width: min(70vw, 280px);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding: 0;
  background: var(--float-bg);
  border: 1px solid var(--glass-border);
  border-top: none;
  border-bottom: none;
  border-radius: 0;
  box-shadow: var(--shadow-lg);
  animation: turn-nav-card-in var(--ease);
}

@keyframes turn-nav-card-in {
  from {
    opacity: 0;
    transform: translateX(8px) scale(0.98);
  }
  to {
    opacity: 1;
    transform: translateX(0) scale(1);
  }
}

.turn-nav-list {
  overflow-y: auto;
  padding: var(--space-1);
  margin: auto 0;
  max-height: 100%;
}

.turn-nav-item {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  width: 100%;
  padding: 3px var(--space-3);
  border: 1px solid transparent;
  border-radius: var(--radius);
  background: transparent;
  color: var(--text-dim);
  font: inherit;
  text-align: left;
  cursor: pointer;
  margin-bottom: 1px;
}

.turn-nav-item:hover {
  background: rgba(var(--accent-rgb), 0.06);
  border-color: rgba(var(--accent-rgb), 0.14);
}

.turn-nav-item.active {
  color: var(--text-bright);
  background: rgba(var(--accent-rgb), 0.12);
  border-color: rgba(var(--accent-rgb), 0.28);
}

.turn-nav-item-index {
  flex-shrink: 0;
  min-width: 2ch;
  text-align: center;
  font-size: var(--font-xs);
  font-weight: 600;
  line-height: 1;
  font-variant-numeric: tabular-nums;
  color: var(--text-faint);
}

.turn-nav-item.active .turn-nav-item-index {
  color: var(--accent);
}

.turn-nav-item-time {
  font-size: var(--font-xs);
  line-height: 1.3;
  color: var(--text-faint);
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
  margin-left: auto;
}

.turn-nav-item.active .turn-nav-item-time {
  color: var(--text-dim);
}

.turn-nav-item-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-size: var(--font-sm);
  line-height: 1.5;
  color: inherit;
}

/* 条目悬停的用户消息气泡预览：贴条目行左侧浮层显示（相对导航根定位 + translate 左伸） */
.turn-nav-preview {
  pointer-events: auto;
  position: absolute;
  left: 0;
  top: 0;
  z-index: 40;
  width: fit-content;
  max-width: min(70vw, 420px);
  max-height: 60vh;
  overflow-y: auto;
  padding: var(--space-3);
  background: var(--float-bg);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-lg);
  transform: translate(-100%, -50%);
}

/* 气泡复用：去除聊天流布局副作用，浮层内贴近整宽展示 */
.turn-nav-preview :deep(.msg) {
  margin: 0;
  animation: none;
  content-visibility: visible;
}

.turn-nav-preview :deep(.msg-user) {
  justify-content: flex-end;
  width: fit-content;
  max-width: min(70vw, 420px);
}

.turn-nav-preview :deep(.msg-user .bubble) {
  width: fit-content;
  max-width: min(70vw, 420px);
  box-shadow: none;
}

.turn-nav-empty {
  padding: var(--space-4);
  color: var(--text-faint);
  font-size: var(--font-sm);
  text-align: center;
}

/* 屏幕阅读器播报区域（视觉隐藏） */
.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
  border: 0;
}

.chat-empty {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
}

/* 回合分组：用户消息起始新回合，组内间距统一由 flex gap 控制 */
.turn {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.turn + .turn {
  margin-top: var(--space-12);
}

/* 回合内条目间距交给 gap，中和各自 legacy 底部 margin */
.turn :deep(.msg) {
  margin-bottom: 0;
}

.turn :deep(.assistant-card) {
  margin-bottom: 0;
}

.turn-highlight :deep(.bubble) {
  animation: turn-highlight-flash 1s ease;
}

@keyframes turn-highlight-flash {
  0%,
  100% {
    box-shadow: var(--inset-shadow), 0 2px 12px rgba(var(--accent-rgb), 0.13);
  }
  30% {
    box-shadow:
      var(--inset-shadow),
      0 0 0 2px rgba(var(--accent-rgb), 0.55),
      0 0 20px rgba(var(--accent-rgb), 0.25);
  }
}

/* 历史会话加载/切回吸底测量：一次性真实渲染全部消息，纠正 content-visibility
 * 估算高度导致的「滚不到最新」；测量后移除，contain-intrinsic-size: auto 记住真实高度 */
.chat-scroll.measuring :deep(.msg) {
  content-visibility: visible;
}

/* 日期分隔线 */
.date-sep {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  text-align: center;
  color: var(--text-faint);
  font-size: var(--font-sm);
  margin: var(--space-1) 0 var(--space-8);
}

.date-sep::before,
.date-sep::after {
  content: "";
  flex: 1;
  height: 1px;
  background: var(--border);
}

.loading-thread-note {
  text-align: center;
  color: var(--text-faint);
  font-size: var(--font-sm);
  padding: var(--space-4) 0;
}
</style>
