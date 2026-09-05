import {
  computed,
  nextTick,
  ref,
  watch,
  type ComputedRef,
  type Ref,
} from "vue";
import type { SessionTab } from "./useCodex";
import type { ThreadItem } from "../lib/types";
import { formatChatTime } from "../lib/format";
import { findCurrentTurnIndex } from "../lib/turns";
import { getUserMessageSummary } from "../lib/userMessage";
import {
  loadPendingImages,
  waitForRenderQuiet,
} from "../lib/chatWarmup";

// 回合跳转动画时长（自绘 rAF 缓动，不使用原生 scrollTo smooth）
const NAV_ANIM_MS = 220;
// 悬停移出导航按钮/卡片后的关闭延迟：覆盖按钮与卡片之间 10px 间隙的穿越防抖
const NAV_HOVER_CLOSE_DELAY_MS = 150;

/** 回合导航与全文预热的接缝（由 ChatView 注入） */
export interface TurnNavDeps {
  scroller: Ref<HTMLElement | null>;
  tab: Ref<SessionTab> | ComputedRef<SessionTab>;
  /** 当前线程消息（userMessage 即导航锚点） */
  items: ComputedRef<ThreadItem[]>;
  /** 吸底状态（当前回合判定：吸底时恒取最新回合） */
  stickToBottom: Ref<boolean>;
  /** 吸底落定流程进行中（预热守卫，避免与强制布局互相踩踏） */
  isSettling(): boolean;
  /** 导航按钮/卡片根元素与卡片列表元素（模板 ref 由调用方绑定） */
  turnNavRoot: Ref<HTMLElement | null>;
  turnCardBody: Ref<HTMLElement | null>;
}

/**
 * 回合定位：导航按钮 + 卡片 + 悬停气泡预览，以 userMessage（data-turn-anchor）为切点。
 * 同时负责「首次跳转前的全文预热」与「跳转自绘动画」两套支撑机制。
 */
export function useTurnNav(deps: TurnNavDeps) {
  const anchorCount = ref(0);
  const currentIndex = ref(-1);
  const turnNavReady = ref(false);
  const turnNavOpen = ref(false);
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
  // 回合跳转动画状态：自绘动画期间锁定索引，连点取消旧动画从当前位置继续
  let navAnimSeq = 0;
  let navAnimRaf: number | undefined;
  let navAnimating = false;

  // ≥2 个回合才提供导航（单回合无跳转意义，按钮不显示）
  const hasTurnNav = computed(
    () => anchorCount.value >= 2 && turnNavReady.value,
  );

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
    for (const item of deps.items.value) {
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
    for (const item of deps.items.value) {
      if (item.id === id) return item.type === "userMessage" ? item : null;
    }
    return null;
  });

  function collectAnchorNodes(): HTMLElement[] {
    return deps.scroller.value
      ? Array.from(
          deps.scroller.value.querySelectorAll<HTMLElement>(
            "[data-turn-anchor]",
          ),
        )
      : [];
  }

  /** 重算锚点内容坐标；吸底状态下当前回合取最新一条，否则按视口顶部判定 */
  function syncAnchors() {
    const el = deps.scroller.value;
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
        : deps.stickToBottom.value
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
   * 首次跳转前的全文预热：measuring 强制真实渲染，图片/异步 Markdown 稳定后再
   * 移除类并等两帧，使 content-visibility 记住全部真实高度。不改变当前 scrollTop。
   */
  async function warmUpHistoryLayout() {
    const el = deps.scroller.value;
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
    if (
      layoutWarmed ||
      warmRunning ||
      deps.isSettling() ||
      deps.tab.value.loading
    ) {
      return;
    }
    const threadId = deps.tab.value.threadId;
    const hasUserMessage = deps.items.value.some(
      (item) => item.type === "userMessage",
    );
    if (!threadId || !hasUserMessage || !deps.scroller.value) return;
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
      deps.isSettling() ||
      deps.tab.value.loading ||
      !deps.items.value.some((item) => item.type === "userMessage")
    ) {
      return;
    }
    void nextTick().then(() => beginBackgroundWarm());
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

  function cancelNavHoverClose() {
    if (navHoverCloseTimer !== undefined) {
      window.clearTimeout(navHoverCloseTimer);
      navHoverCloseTimer = undefined;
    }
  }

  function hideTurnPreview() {
    cancelPreviewHide();
    previewItemId.value = null;
    previewX.value = -9999;
    previewY.value = -9999;
  }

  function closeTurnNav() {
    cancelNavHoverClose();
    hideTurnPreview();
    turnNavOpen.value = false;
  }

  async function openTurnNav() {
    turnNavOpen.value = true;
    await nextTick();
    const active = deps.turnCardBody.value?.querySelector<HTMLElement>(
      ".turn-nav-item.active",
    );
    active?.scrollIntoView?.({ block: "nearest" });
  }

  /** 悬停进入按钮/卡片：未展开时立即展开（纯悬停开关，不固定） */
  function onTurnNavEnter() {
    cancelNavHoverClose();
    if (!turnNavOpen.value) void openTurnNav();
  }

  /** 悬停移出按钮/卡片：延迟关闭卡片，期间重新进入即取消 */
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
    const rootRect = deps.turnNavRoot.value?.getBoundingClientRect();
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

  function onGlobalPointerDown(e: PointerEvent) {
    const target = e.target as Node | null;
    if (target && deps.turnNavRoot.value?.contains(target)) return;
    closeTurnNav();
  }

  function onGlobalKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") closeTurnNav();
  }

  watch(turnNavOpen, (open) => {
    if (open) {
      document.addEventListener("pointerdown", onGlobalPointerDown);
      document.addEventListener("keydown", onGlobalKeyDown);
    } else {
      document.removeEventListener("pointerdown", onGlobalPointerDown);
      document.removeEventListener("keydown", onGlobalKeyDown);
    }
  });

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

  /** 直接滚到指定用户消息回合起点（顶部对齐），并短暂高亮目标气泡 */
  async function jumpToTurn(target: number) {
    const el = deps.scroller.value;
    if (!el || target < 0 || target >= anchorCount.value) return;
    // 手动浏览历史：解除吸底，避免 MutationObserver/流式更新把位置拉回
    deps.stickToBottom.value = false;
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

  /** 可信用户滚动时更新当前回合索引（由吸底滚动引擎回调） */
  function onTrustedScroll(scrollTop: number) {
    currentIndex.value = findCurrentTurnIndex(anchorTops, scrollTop);
  }

  /** 切换线程/重新加载时复位导航与预热状态 */
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

  /** 挂载后建立尺寸观察：窗口/容器尺寸变化时重算锚点 */
  function attachResizeObserver() {
    if (deps.scroller.value && typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(() => scheduleAnchorSync());
      resizeObserver.observe(deps.scroller.value);
    }
    scheduleAnchorSync();
    queueBackgroundWarm();
  }

  function dispose() {
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
  }

  return {
    anchorCount,
    currentIndex,
    turnNavReady,
    turnNavOpen,
    hasTurnNav,
    turnEntries,
    previewItem,
    previewX,
    previewY,
    isNavAnimating: () => navAnimating,
    onTrustedScroll,
    syncAnchors,
    scheduleAnchorSync,
    beginBackgroundWarm,
    queueBackgroundWarm,
    resetTurnNav,
    resetWarmState,
    selectTurn,
    jumpToTurn,
    onTurnNavEnter,
    onTurnNavLeave,
    onTurnNavKeydown,
    onTurnItemEnter,
    onTurnItemLeave,
    onTurnPreviewEnter,
    attachResizeObserver,
    dispose,
  };
}
