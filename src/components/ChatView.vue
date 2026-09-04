<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import ComposerBar from "./ComposerBar.vue";
import EmptyState from "./EmptyState.vue";
import InlineInteraction from "./InlineInteraction.vue";
import MessageItem from "./MessageItem.vue";
import PlanCard from "./PlanCard.vue";
import PlanPromptBubble from "./PlanPromptBubble.vue";
import { store, type SessionTab } from "../composables/useCodex";
import { useContextUsage } from "../composables/useContextUsage";
import { formatChatTime, formatTokens } from "../lib/format";
import {
  ICON_ARROW_DOWN,
  ICON_ARROW_UP,
  ICON_COMPRESS,
  ICON_GRID_4,
} from "../lib/icons";
import {
  createTurnsBuilder,
  findCurrentTurnIndex,
  type Turn,
} from "../lib/turns";
import { getUserMessageSummary } from "../lib/userMessage";

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

// 是否吸附在底部：用户上滑查看历史时暂停自动滚动
const stickToBottom = ref(true);
// 最近一次程序化吸底写入/滚动事件后的 scrollTop：
// 用户可信滚动相对该位置向上移动即视为“上滑看历史”，一次正常上滑立即解除；
// 向下（追赶窗口内）或程序化滚动不会误解除。
let lastStickScrollTop = 0;
// 解除吸底所需的最小距底距离：轻微误触/抖动（< 32px）不解除，
// 单次正常滚轮（约 80-100px）一次即解除
const UNPIN_DIST_PX = 32;
// 首次跳转前的全文预热参数：渲染安静期与整体硬上限
const WARM_QUIET_MS = 120;
const WARM_MAX_MS = 2000;
// 回合跳转动画时长（自绘 rAF 缓动，不使用原生 scrollTo smooth）
const NAV_ANIM_MS = 220;
// 悬停移出导航按钮/卡片后的关闭延迟：覆盖按钮与卡片之间 10px 间隙的穿越防抖
const NAV_HOVER_CLOSE_DELAY_MS = 150;
// 是否有正在流式输出或进行中的工具/命令（useCodex 按线程增量维护）
const hasActiveWork = computed(
  () => (store.activeWorkByThread[props.tab.threadId ?? ""] ?? 0) > 0,
);

// ---------- 回合定位：导航按钮 + 卡片，以 userMessage（data-turn-anchor）为切点 ----------
const anchorCount = ref(0);
const currentIndex = ref(-1);
const turnNavReady = ref(false);
const turnNavOpen = ref(false);
const turnNavRoot = ref<HTMLElement | null>(null);
const turnCardBody = ref<HTMLElement | null>(null);
const highlightTimer = ref<number | undefined>(undefined);
let navHoverCloseTimer: number | undefined;
// 悬停条目时展示的用户消息气泡预览
const previewItemId = ref<string | null>(null);
const previewX = ref(-9999);
const previewY = ref(-9999);
let previewHideTimer: number | undefined;
let anchorTops: number[] = [];
let anchorSyncScheduled = false;
let anchorSyncRaf: number | undefined;
let resizeObserver: ResizeObserver | undefined;
let highlightedNode: HTMLElement | null = null;
// 本线程是否已完成“全文真实布局”预热：完成后才显示导航按钮
let layoutWarmed = false;
let warmRunning = false;
let warmSeq = 0;
let settleRunning = false;
// 回合跳转动画状态：自绘动画期间锁定索引，连点取消旧动画从当前位置继续
let navAnimSeq = 0;
let navAnimRaf: number | undefined;
let navAnimating = false;

const hasTurnNav = computed(
  () => anchorCount.value >= 2 && turnNavReady.value,
);

// ---------- 合并信息簇：上下文占用 / token 用量（回合导航卡片头部） ----------
const { ctxUsage, compacting, compactNow } = useContextUsage(props.tab);

/** 本会话累计输入/输出 token（导航头部展示）；缺任一不显示 */
const tokenTotals = computed(() => {
  const u = props.tab.threadTokenUsage;
  if (!u || typeof u.input !== "number" || typeof u.output !== "number") {
    return null;
  }
  return { input: u.input, output: u.output };
});

/** 卡片条目：按消息顺序收集 userMessage，index 即 DOM 锚点顺序 */
const turnEntries = computed(() => {
  const out: {
    id: string;
    index: number;
    /** 完整标题源（CSS 单行省略负责视觉截断）：普通消息为全文，执行计划消息为计划标题 */
    title: string;
    time: string;
  }[] = [];
  let idx = -1;
  for (const item of items.value) {
    if (item.type !== "userMessage") continue;
    idx++;
    // 执行计划消息（气泡呈计划卡片）以计划标题为导航标题，其余沿用完整纯文本预览
    const summary = getUserMessageSummary(item);
    const time =
      typeof item.startedAtMs === "number"
        ? formatChatTime(item.startedAtMs)
        : "";
    out.push({
      id: item.id,
      index: idx,
      title: summary.isExecutePlan ? summary.planTitle : summary.navText,
      time,
    });
  }
  return out;
});

/** 当前悬停条目的用户消息（供气泡预览浮层渲染） */
const previewItem = computed(() => {
  const id = previewItemId.value;
  if (!id) return null;
  for (const item of items.value) {
    if (item.id === id) return item.type === "userMessage" ? item : null;
  }
  return null;
});

function collectAnchorNodes(): HTMLElement[] {
  return scroller.value
    ? Array.from(
        scroller.value.querySelectorAll<HTMLElement>("[data-turn-anchor]"),
      )
    : [];
}

/** 重算锚点内容坐标；吸底状态下当前回合取最新一条，否则按视口顶部判定 */
function syncAnchors() {
  const el = scroller.value;
  if (!el) return;
  const nodes = collectAnchorNodes();
  const containerTop = el.getBoundingClientRect().top;
  anchorTops = nodes.map(
    (node) => node.getBoundingClientRect().top - containerTop + el.scrollTop,
  );
  anchorCount.value = nodes.length;
  currentIndex.value =
    nodes.length === 0
      ? -1
      : stickToBottom.value
        ? nodes.length - 1
        : findCurrentTurnIndex(anchorTops, el.scrollTop);
}

/** 消息/布局变化后节流重算锚点：等 DOM 更新完成再取坐标 */
function scheduleAnchorSync() {
  if (anchorSyncScheduled) return;
  anchorSyncScheduled = true;
  void nextTick().then(() => {
    if (!anchorSyncScheduled) return;
    anchorSyncRaf = requestAnimationFrame(() => {
      anchorSyncScheduled = false;
      anchorSyncRaf = undefined;
      syncAnchors();
    });
  });
}

function clearTurnHighlight() {
  if (highlightedNode) {
    highlightedNode.classList.remove("turn-highlight");
    highlightedNode = null;
  }
  if (highlightTimer.value !== undefined) {
    window.clearTimeout(highlightTimer.value);
    highlightTimer.value = undefined;
  }
}

/** 目标用户气泡短暂高亮约 1 秒，便于确认落点 */
function flashTurn(node: HTMLElement) {
  clearTurnHighlight();
  node.classList.add("turn-highlight");
  highlightedNode = node;
  highlightTimer.value = window.setTimeout(() => {
    clearTurnHighlight();
  }, 1000);
}

/**
 * 等待 Markdown Worker / 代码块装饰等异步 DOM 落地：
 * 连续 WARM_QUIET_MS 无子树变化即认为稳定，WARM_MAX_MS 硬上限兜底。
 */
function waitForRenderQuiet(el: HTMLElement): Promise<void> {
  return new Promise((resolve) => {
    if (typeof MutationObserver === "undefined") {
      window.setTimeout(resolve, WARM_QUIET_MS);
      return;
    }
    let settled = false;
    let quietTimer: number | undefined;
    let hardTimer: number | undefined;
    const finish = () => {
      if (settled) return;
      settled = true;
      if (quietTimer !== undefined) window.clearTimeout(quietTimer);
      if (hardTimer !== undefined) window.clearTimeout(hardTimer);
      observer.disconnect();
      resolve();
    };
    const kickQuiet = () => {
      if (quietTimer !== undefined) window.clearTimeout(quietTimer);
      quietTimer = window.setTimeout(finish, WARM_QUIET_MS);
    };
    const observer = new MutationObserver(kickQuiet);
    observer.observe(el, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "style"],
    });
    kickQuiet();
    hardTimer = window.setTimeout(finish, WARM_MAX_MS);
  });
}

/** 临时把仍为懒加载的图片置为 eager 并等待解码，完成后恢复 lazy */
async function loadPendingImages(el: HTMLElement): Promise<void> {
  const pending = Array.from(
    el.querySelectorAll<HTMLImageElement>("img"),
  ).filter((img) => !img.complete);
  if (pending.length === 0) return;
  const eager: HTMLImageElement[] = [];
  for (const img of pending) {
    if (img.loading === "lazy") {
      eager.push(img);
      img.loading = "eager";
    }
  }
  try {
    await Promise.race([
      Promise.allSettled(
        pending.map((img) =>
          img.complete
            ? Promise.resolve()
            : typeof img.decode === "function"
              ? img.decode().catch(() => undefined)
              : Promise.resolve(),
        ),
      ),
      new Promise<void>((resolve) => {
        window.setTimeout(resolve, WARM_MAX_MS);
      }),
    ]);
  } finally {
    for (const img of eager) img.loading = "lazy";
  }
}

/**
 * 首次跳转前的全文预热：measuring 强制真实渲染，图片/异步 Markdown 稳定后再
 * 移除类并等两帧，使 content-visibility 记住全部真实高度。不改变当前 scrollTop。
 */
async function warmUpHistoryLayout() {
  const el = scroller.value;
  if (!el) return;
  el.classList.add("measuring");
  try {
    await nextTick();
    await Promise.all([loadPendingImages(el), waitForRenderQuiet(el)]);
  } finally {
    el.classList.remove("measuring");
  }
  await nextTick();
  await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
}

/** 作废当前线程的预热结果（切换线程/重新加载时调用） */
function resetWarmState() {
  warmSeq++;
  warmRunning = false;
  layoutWarmed = false;
  turnNavReady.value = false;
  turnNavOpen.value = false;
}

/**
 * 会话打开后在后台执行一次全文预热；完成前不显示导航按钮。
 * 预热按线程一次性执行，warmSeq/threadId 守卫防止旧线程结果误落新线程。
 */
function beginBackgroundWarm() {
  if (layoutWarmed || warmRunning || settleRunning || props.tab.loading) return;
  const threadId = props.tab.threadId;
  const hasUserMessage = items.value.some((item) => item.type === "userMessage");
  if (!threadId || !hasUserMessage || !scroller.value) return;
  warmRunning = true;
  const seq = warmSeq;
  void warmUpHistoryLayout()
    .catch(() => undefined)
    .finally(() => {
      if (seq !== warmSeq) return;
      warmRunning = false;
      layoutWarmed = true;
      turnNavReady.value = true;
      scheduleAnchorSync();
    });
}

/** 内容刚出现且暂无可执行预热时机时，等 DOM 渲染后补一次后台预热 */
function queueBackgroundWarm() {
  if (
    layoutWarmed ||
    warmRunning ||
    settleRunning ||
    props.tab.loading ||
    !items.value.some((item) => item.type === "userMessage")
  ) {
    return;
  }
  void nextTick().then(() => beginBackgroundWarm());
}

function resetTurnNav() {
  resetWarmState();
  cancelTurnAnimation();
  clearTurnHighlight();
  cancelNavHoverClose();
  hideTurnPreview();
  anchorTops = [];
  anchorCount.value = 0;
  currentIndex.value = -1;
}

/** 取消正在进行的回合跳转动画（seq 递增使旧 rAF 回调失效） */
function cancelTurnAnimation() {
  navAnimSeq++;
  navAnimating = false;
  if (navAnimRaf !== undefined) {
    cancelAnimationFrame(navAnimRaf);
    navAnimRaf = undefined;
  }
}

function closeTurnNav() {
  cancelNavHoverClose();
  hideTurnPreview();
  turnNavOpen.value = false;
}

async function openTurnNav() {
  turnNavOpen.value = true;
  await nextTick();
  const active = turnCardBody.value?.querySelector<HTMLElement>(
    ".turn-nav-item.active",
  );
  active?.scrollIntoView?.({ block: "nearest" });
}

function cancelNavHoverClose() {
  if (navHoverCloseTimer !== undefined) {
    window.clearTimeout(navHoverCloseTimer);
    navHoverCloseTimer = undefined;
  }
}

/** 悬停进入按钮/卡片：未展开时立即展开（纯悬停开关，不固定） */
function onTurnNavEnter() {
  cancelNavHoverClose();
  if (!turnNavOpen.value) void openTurnNav();
}

/** 悬停移出按钮/卡片：延迟关闭，期间重新进入即取消 */
function onTurnNavLeave() {
  if (!turnNavOpen.value) return;
  cancelNavHoverClose();
  navHoverCloseTimer = window.setTimeout(() => {
    navHoverCloseTimer = undefined;
    hideTurnPreview();
    turnNavOpen.value = false;
  }, NAV_HOVER_CLOSE_DELAY_MS);
}

/** 键盘路径（Enter/Space）：鼠标点击不再切换，仅无 hover 场景的激活方式 */
function toggleTurnNav() {
  cancelNavHoverClose();
  if (turnNavOpen.value) closeTurnNav();
  else void openTurnNav();
}

function onTurnNavKeydown(e: KeyboardEvent) {
  if (e.key !== "Enter" && e.key !== " ") return;
  e.preventDefault();
  toggleTurnNav();
}

function selectTurn(index: number) {
  closeTurnNav();
  void jumpToTurn(index);
}

function cancelPreviewHide() {
  if (previewHideTimer !== undefined) {
    window.clearTimeout(previewHideTimer);
    previewHideTimer = undefined;
  }
}

/** 悬停条目：立即在条目行左侧显示该用户消息的气泡预览 */
function onTurnItemEnter(e: MouseEvent, entry: { id: string }) {
  cancelPreviewHide();
  const row = e.currentTarget as HTMLElement;
  previewItemId.value = entry.id;
  const rect = row.getBoundingClientRect();
  const rootRect = turnNavRoot.value?.getBoundingClientRect();
  if (!rootRect) return;
  // 预览右缘对齐条目行左侧留 8px；坐标换算为相对 .turn-nav 根容器（含 transform）
  previewX.value = rect.left - 8 - rootRect.left;
  previewY.value = rect.top + rect.height / 2 - rootRect.top;
}

/** 移出条目：短暂延迟后隐藏（覆盖移向预览浮层的穿越，进入预览即取消） */
function onTurnItemLeave() {
  cancelPreviewHide();
  previewHideTimer = window.setTimeout(() => {
    previewHideTimer = undefined;
    hideTurnPreview();
  }, NAV_HOVER_CLOSE_DELAY_MS);
}

/** 移入预览浮层：取消条目移出触发的隐藏 */
function onTurnPreviewEnter() {
  cancelPreviewHide();
}

function hideTurnPreview() {
  cancelPreviewHide();
  previewItemId.value = null;
  previewX.value = -9999;
  previewY.value = -9999;
}

function onGlobalPointerDown(e: PointerEvent) {
  const target = e.target as Node | null;
  if (target && turnNavRoot.value?.contains(target)) return;
  closeTurnNav();
}

function onGlobalKeyDown(e: KeyboardEvent) {
  if (e.key === "Escape") closeTurnNav();
}

/**
 * 自绘 ease-out 短动画：每帧写 scrollTop，结束帧强制落到目标并同步索引。
 * 不用原生 scrollTo smooth / scrollend，避免 trusted scroll 事件时序干扰；
 * 同步 rAF（测试/异常环境）两帧时间戳不推进时直接收尾，防死循环。
 */
function startAnimatedScroll(el: HTMLElement, top: number) {
  cancelTurnAnimation();
  const seq = navAnimSeq;
  navAnimating = true;
  const from = el.scrollTop;
  const delta = top - from;
  const easeOutCubic = (t: number) => 1 - (1 - t) ** 3;
  const finish = () => {
    if (seq !== navAnimSeq) return;
    el.scrollTop = top;
    navAnimating = false;
    navAnimRaf = undefined;
    syncAnchors();
  };
  if (Math.abs(delta) < 1) {
    finish();
    return;
  }
  let start: number | undefined;
  let prevNow = -1;
  let sameFrames = 0;
  const step = (now: number) => {
    if (seq !== navAnimSeq) return;
    if (start === undefined) start = now;
    const p = Math.min(1, (now - start) / NAV_ANIM_MS);
    el.scrollTop = from + delta * easeOutCubic(p);
    if (now === prevNow) sameFrames++;
    else {
      sameFrames = 0;
      prevNow = now;
    }
    if (p >= 1 || sameFrames >= 1) {
      finish();
      return;
    }
    navAnimRaf = requestAnimationFrame(step);
  };
  navAnimRaf = requestAnimationFrame(step);
}

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
  // 自绘跳转动画期间的滚动事件不参与索引/吸底推导，结束后由 syncAnchors 收敛
  if (navAnimating) return;
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
  // 仅可信用户滚动参与回合判定；直接定位产生的 scroll 事件会在结束后
  // 把当前回合自然收敛到目标位置
  if (e.isTrusted) {
    currentIndex.value = findCurrentTurnIndex(anchorTops, el.scrollTop);
  }
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
  scheduleAnchorSync();
}

/** 直接滚到指定用户消息回合起点（顶部对齐），并短暂高亮目标气泡 */
async function jumpToTurn(target: number) {
  const el = scroller.value;
  if (!el || target < 0 || target >= anchorCount.value) return;
  // 手动浏览历史：解除吸底，避免 MutationObserver/流式更新把位置拉回
  stickToBottom.value = false;
  currentIndex.value = target;
  // 导航按钮只在后台全文预热完成后出现；此处直接强制真实布局量取精确坐标
  // measuring 强制真实布局后量取内容坐标；移除类并稳定后再自绘动画到目标
  el.classList.add("measuring");
  let node: HTMLElement | null = null;
  let top = 0;
  try {
    await nextTick();
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    const nodes = collectAnchorNodes();
    if (target >= nodes.length) return;
    const containerTop = el.getBoundingClientRect().top;
    anchorTops = nodes.map(
      (node) => node.getBoundingClientRect().top - containerTop + el.scrollTop,
    );
    anchorCount.value = nodes.length;
    top = anchorTops[target];
    node = nodes[target];
  } finally {
    el.classList.remove("measuring");
  }
  if (!node) return;
  await nextTick();
  startAnimatedScroll(el, top);
  flashTurn(node);
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
  settleRunning = true;
  el.classList.add("measuring");
  try {
    await nextTick();
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    el.scrollTop = el.scrollHeight;
    lastStickScrollTop = el.scrollTop;
  } finally {
    el.classList.remove("measuring");
    settleRunning = false;
  }
  scheduleScroll();
  scheduleAnchorSync();
  beginBackgroundWarm();
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

// 切换会话（新建/打开历史/会话失效重置）后重置吸底状态，
// 避免旧会话的上滑状态残留导致新对话默认不吸底
watch(
  () => props.tab.threadId,
  () => {
    resetTurnNav();
    // 切回已加载会话：内容可能仍处 content-visibility 估算布局，直接吸底会停在最新消息之上
    if (items.value.length > 0) {
      void settleToBottom();
    } else {
      stickToBottom.value = true;
      lastStickScrollTop = 0;
      scheduleScroll();
      scheduleAnchorSync();
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

watch(turnNavOpen, (open) => {
  if (open) {
    document.addEventListener("pointerdown", onGlobalPointerDown);
    document.addEventListener("keydown", onGlobalKeyDown);
  } else {
    document.removeEventListener("pointerdown", onGlobalPointerDown);
    document.removeEventListener("keydown", onGlobalKeyDown);
  }
});

// 懒加载/异步图片加载后高度变化没有 DOM 结构变更，补一次跟随
function onImageLoad(e: Event) {
  if ((e.target as Element | null)?.tagName === "IMG") {
    scheduleScroll();
    scheduleAnchorSync();
  }
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
      scheduleAnchorSync();
    });
    scrollObserver.observe(scroller.value, { childList: true, subtree: true });
    scroller.value.addEventListener("load", onImageLoad, true);
    lastStickScrollTop = scroller.value.scrollTop;
  }
  if (scroller.value && typeof ResizeObserver !== "undefined") {
    resizeObserver = new ResizeObserver(() => scheduleAnchorSync());
    resizeObserver.observe(scroller.value);
  }
  scheduleScroll();
  scheduleAnchorSync();
  queueBackgroundWarm();
});

onBeforeUnmount(() => {
  if (scrollRaf !== undefined) cancelAnimationFrame(scrollRaf);
  scrollRaf = undefined;
  cancelTurnAnimation();
  cancelNavHoverClose();
  cancelPreviewHide();
  document.removeEventListener("pointerdown", onGlobalPointerDown);
  document.removeEventListener("keydown", onGlobalKeyDown);
  anchorSyncScheduled = false;
  if (anchorSyncRaf !== undefined) cancelAnimationFrame(anchorSyncRaf);
  anchorSyncRaf = undefined;
  resizeObserver?.disconnect();
  resizeObserver = undefined;
  clearTurnHighlight();
  scrollObserver?.disconnect();
  scrollObserver = undefined;
  scroller.value?.removeEventListener("load", onImageLoad, true);
  // 避免切换视图后残留滚动状态
  stickToBottom.value = true;
});
</script>

<template>
  <div class="chat">
    <div class="chat-scroll-wrap">
      <div ref="scroller" class="chat-scroll" @scroll="onScroll">
        <div v-if="items.length === 0" class="chat-empty">
          <EmptyState :busy="tab.turnActive || tab.creatingChat" />
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
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_GRID_4" />
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
          <div v-if="tokenTotals || ctxUsage" class="turn-nav-foot">
            <span
              v-if="tokenTotals"
              class="token-usage-chip"
              v-tooltip="`输入 ${formatTokens(tokenTotals.input)} · 输出 ${formatTokens(tokenTotals.output)}`"
              :aria-label="`输入 ${formatTokens(tokenTotals.input)} · 输出 ${formatTokens(tokenTotals.output)}`"
            >
              <span class="token-usage-part">
                <svg class="token-usage-ico" viewBox="0 0 24 24" aria-hidden="true">
                  <path :d="ICON_ARROW_UP" />
                </svg>
                {{ formatTokens(tokenTotals.input) }}
              </span>
              <span class="token-usage-sep">·</span>
              <span class="token-usage-part">
                <svg class="token-usage-ico" viewBox="0 0 24 24" aria-hidden="true">
                  <path :d="ICON_ARROW_DOWN" />
                </svg>
                {{ formatTokens(tokenTotals.output) }}
              </span>
            </span>
            <span
              v-if="ctxUsage"
              class="ctx-control-capsule"
            >
              <span
                class="ctx-usage-text"
                v-tooltip="`上下文已用 ${formatTokens(ctxUsage.used)}，共 ${formatTokens(ctxUsage.window)}`"
                :aria-label="`上下文已用 ${formatTokens(ctxUsage.used)}，共 ${formatTokens(ctxUsage.window)}`"
              >
                {{ formatTokens(ctxUsage.used) }} / {{ formatTokens(ctxUsage.window) }}
              </span>
              <button
                type="button"
                class="ctx-compact-btn"
                aria-label="压缩上下文"
                v-tooltip="'压缩上下文'"
                :disabled="!props.tab.threadId || compacting"
                @click="compactNow()"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path :d="ICON_COMPRESS" />
                </svg>
              </button>
            </span>
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
