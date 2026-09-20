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
import { sessionLog } from "../lib/sessionLog";

// 回合跳转动画时长（自绘 rAF 缓动，不使用原生 scrollTo smooth）
const NAV_ANIM_MS = 220;
// 悬停移出导航按钮/卡片后的关闭延迟：覆盖按钮与卡片之间 10px 间隙的穿越防抖
const NAV_HOVER_CLOSE_DELAY_MS = 150;
// 跳转落地后的跟随收敛：窗口上限、单帧容差、判定稳定所需的连续帧数
const NAV_SETTLE_MS = 1500;
const NAV_SETTLE_TOL_PX = 1;
const NAV_SETTLE_STABLE_FRAMES = 3;
// 首次落点偏差超过该值才写诊断日志（正常跳转不写，避免日志噪声）
const NAV_DRIFT_LOG_PX = 2;
// 预热未稳定时的重试间隔与次数上限
const WARM_RETRY_MS = 1000;
const WARM_RETRY_MAX = 5;
// 用户消息锚点属性：值为消息 id，跳转按身份定位而非序数
const TURN_ANCHOR_ATTR = "data-turn-anchor";

/** 导航条目（卡片点击传入）：id 为目标用户消息身份，index 为锚点序数 */
export interface TurnNavTarget {
  id: string;
  index: number;
}

/** 单调时钟（performance 不可用时回落到 Date） */
function nowMs(): number {
  return typeof performance !== "undefined" &&
    typeof performance.now === "function"
    ? performance.now()
    : Date.now();
}

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
  /** 登记跳转动画的程序化落点（吸底引擎据此识别尾随 scroll 事件，避免误判为用户滚动） */
  onProgrammaticScroll(top: number): void;
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
  let warmRetryTimer: number | undefined;
  let warmRetryCount = 0;
  // 回合跳转窗口状态：从点击起（含 measuring/nextTick 等待）到动画结束的下一帧都置位，
  // 期间到达的 scroll 事件一律视为程序化滚动（不参与吸底/索引推导）；
  // 连点取消旧动画从当前位置继续
  let navAnimSeq = 0;
  let navAnimRaf: number | undefined;
  let navAnimating = false;
  // measuring 类引用计数：跳转量取与预热可能重叠，避免提前摘掉强制渲染
  let measuringDepth = 0;
  // 跳转窗口内挂载用户输入监听的目标元素（取消跟随用）
  let jumpGuardEl: HTMLElement | null = null;

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
            `[${TURN_ANCHOR_ATTR}]`,
          ),
        )
      : [];
  }

  /** 强制真实渲染类（引用计数：跳转量取与预热重叠时不互相摘除） */
  function addMeasuring(el: HTMLElement) {
    measuringDepth++;
    el.classList.add("measuring");
  }

  function removeMeasuring(el: HTMLElement) {
    measuringDepth = Math.max(0, measuringDepth - 1);
    if (measuringDepth === 0) el.classList.remove("measuring");
  }

  /**
   * 目标锚点：优先按 id 匹配（index 与 DOM 锚点顺序一旦错位就会跳到别的消息，
   * 按身份匹配可免疫），匹配不到再用序数兜底并由调用方记日志。
   */
  function resolveAnchor(
    nodes: HTMLElement[],
    entry: TurnNavTarget,
  ): { node: HTMLElement; index: number; byId: boolean } | null {
    for (let i = 0; i < nodes.length; i++) {
      if (nodes[i].getAttribute(TURN_ANCHOR_ATTR) === entry.id) {
        return { node: nodes[i], index: i, byId: true };
      }
    }
    const node = nodes[entry.index] ?? null;
    return node ? { node, index: entry.index, byId: false } : null;
  }

  /** 目标锚点相对滚动容器顶边的实时偏差：>0 表示目标在视口顶部之下（需向下滚这么多） */
  function anchorDrift(el: HTMLElement, node: HTMLElement): number {
    return node.getBoundingClientRect().top - el.getBoundingClientRect().top;
  }

  /** 跳转诊断日志（只记安全数值字段，失败静默） */
  function logJump(event: string, detail: string) {
    void sessionLog("warn", deps.tab.value.threadId, event, detail);
  }

  /** 跳转窗口内的用户真实输入：立即让位（取消动画/跟随并摘掉监听） */
  function onJumpUserInput(e: Event) {
    if (!e.isTrusted) return;
    cancelTurnAnimation();
    detachJumpGuards();
  }

  function attachJumpGuards(el: HTMLElement) {
    if (jumpGuardEl) return;
    jumpGuardEl = el;
    el.addEventListener("wheel", onJumpUserInput, { passive: true });
    el.addEventListener("pointerdown", onJumpUserInput, true);
    el.addEventListener("touchstart", onJumpUserInput, { passive: true });
    window.addEventListener("keydown", onJumpUserInput, true);
  }

  function detachJumpGuards() {
    const el = jumpGuardEl;
    if (!el) return;
    jumpGuardEl = null;
    el.removeEventListener("wheel", onJumpUserInput);
    el.removeEventListener("pointerdown", onJumpUserInput, true);
    el.removeEventListener("touchstart", onJumpUserInput);
    window.removeEventListener("keydown", onJumpUserInput, true);
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
   * 返回本次预热结束时布局是否真正稳定（超时返回 false，调用方不得当成已稳定）。
   */
  async function warmUpHistoryLayout(): Promise<boolean> {
    const el = deps.scroller.value;
    if (!el) return false;
    let stable = false;
    addMeasuring(el);
    try {
      await nextTick();
      const [, quiet] = await Promise.all([
        loadPendingImages(el),
        waitForRenderQuiet(el),
      ]);
      stable = quiet.stable;
    } finally {
      removeMeasuring(el);
    }
    await nextTick();
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    return stable;
  }

  /** 作废当前线程的预热结果（切换线程/重新加载时调用） */
  function resetWarmState() {
    warmSeq++;
    warmRunning = false;
    layoutWarmed = false;
    turnNavReady.value = false;
    turnNavOpen.value = false;
    clearWarmRetry();
    warmRetryCount = 0;
  }

  function clearWarmRetry() {
    if (warmRetryTimer !== undefined) {
      window.clearTimeout(warmRetryTimer);
      warmRetryTimer = undefined;
    }
  }

  /** 预热未稳定时的延迟重试（seq 守卫：切线程/重载后旧重试不生效） */
  function scheduleWarmRetry(seq: number) {
    if (warmRetryCount >= WARM_RETRY_MAX) return;
    warmRetryCount++;
    clearWarmRetry();
    warmRetryTimer = window.setTimeout(() => {
      warmRetryTimer = undefined;
      if (seq !== warmSeq) return;
      beginBackgroundWarm();
    }, WARM_RETRY_MS);
  }

  /**
   * 会话打开后在后台执行全文预热：首次尝试结束即显示导航按钮（可见时机不变），
   * 但只有真正稳定（DOM 安静且渲染队列清空）才算预热完成；未稳定时按间隔重试，
   * 期间跳转由落地后的跟随收敛兜底。warmSeq/threadId 守卫防止旧线程结果误落新线程。
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
    void (async () => {
      let stable = false;
      try {
        stable = await warmUpHistoryLayout();
      } catch {
        stable = false;
      }
      if (seq !== warmSeq) return;
      warmRunning = false;
      // 只有「DOM 安静且渲染队列清空」才算预热完成；超时不再被当成已稳定
      if (stable) layoutWarmed = true;
      // 按钮可见时机保持现状：首次预热尝试结束即显示
      turnNavReady.value = true;
      scheduleAnchorSync();
      if (!stable) scheduleWarmRetry(seq);
    })();
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

  function selectTurn(entry: TurnNavTarget) {
    closeTurnNav();
    void jumpToTurn(entry);
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

  /** 提前返回时关窗：新跳转已接管（seq 变化）则不动，避免误关新窗口 */
  function endJumpWindow(seq: number) {
    if (seq === navAnimSeq) navAnimating = false;
  }

  /**
   * 量取目标锚点：measuring 强制真实布局后取内容坐标，并按 id 解析目标节点。
   * 返回 "cancelled" 表示等待期间已被取消（新跳转/切线程），调用方应直接放弃。
   */
  async function measureAnchor(
    el: HTMLElement,
    entry: TurnNavTarget,
    seq: number,
  ): Promise<
    { node: HTMLElement; top: number; byId: boolean } | "cancelled" | null
  > {
    addMeasuring(el);
    try {
      await nextTick();
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      if (seq !== navAnimSeq) return "cancelled";
      const nodes = collectAnchorNodes();
      const containerTop = el.getBoundingClientRect().top;
      anchorTops = nodes.map(
        (node) => node.getBoundingClientRect().top - containerTop + el.scrollTop,
      );
      anchorCount.value = nodes.length;
      const hit = resolveAnchor(nodes, entry);
      return hit
        ? { node: hit.node, top: anchorTops[hit.index], byId: hit.byId }
        : null;
    } finally {
      removeMeasuring(el);
    }
  }

  /**
   * 自绘 ease-out 短动画：每帧写 scrollTop，结束帧强制落到目标并同步索引后回调 onDone。
   * 不用原生 scrollTo smooth / scrollend，避免 trusted scroll 事件时序干扰；
   * 同步 rAF（测试/异常环境）两帧时间戳不推进时直接收尾，防死循环。
   */
  function animateScrollTo(
    el: HTMLElement,
    top: number,
    seq: number,
    onDone: () => void,
  ) {
    const from = el.scrollTop;
    const delta = top - from;
    const easeOutCubic = (t: number) => 1 - (1 - t) ** 3;
    const finish = () => {
      navAnimRaf = undefined;
      if (seq !== navAnimSeq) return;
      el.scrollTop = top;
      // 登记落点并与落点收敛索引
      deps.onProgrammaticScroll(top);
      syncAnchors();
      onDone();
    };
    if (Math.abs(delta) < 1) {
      finish();
      return;
    }
    let start: number | undefined;
    let prevNow = -1;
    let sameFrames = 0;
    const step = (now: number) => {
      if (seq !== navAnimSeq) {
        navAnimRaf = undefined;
        return;
      }
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

  /**
   * 落地后的跟随收敛：历史会话打开时正文（Markdown Worker 渲染、代码高亮、图片）仍在
   * 陆续落地，量取时刻的高度与最终高度不同，一次性写入的绝对落点必然漂移。
   * 这里按实时布局把目标顶边持续对齐容器顶边，直到连续稳定或窗口上限；
   * 每次读几何都走实时布局（不重复加 measuring）：既避免大历史下每帧强制全量真实布局，
   * 又让「测量」与「滚动」始终处于同一套布局，即使浏览器对记忆尺寸的处理有差异也收敛。
   * 被取消（用户输入/新跳转/切线程）或目标节点已卸载时不回调收尾。
   */
  function followAnchorToTop(
    el: HTMLElement,
    node: HTMLElement,
    seq: number,
    onEnd: (settled: boolean, drift: number, rounds: number) => void,
  ) {
    const startedAt = nowMs();
    let rounds = 0;
    let stableFrames = 0;
    const step = () => {
      navAnimRaf = undefined;
      if (seq !== navAnimSeq) return;
      // 目标已不在聊天流里（消息替换/线程切换）：放弃跟随，不做收尾高亮
      if (!el.contains(node)) {
        onEnd(false, 0, rounds);
        return;
      }
      const drift = anchorDrift(el, node);
      if (Math.abs(drift) <= NAV_SETTLE_TOL_PX) {
        stableFrames++;
        if (stableFrames >= NAV_SETTLE_STABLE_FRAMES) {
          onEnd(true, drift, rounds);
          return;
        }
      } else {
        stableFrames = 0;
        // 按残余偏差直接微调：位移小、频率低，观感仍是平滑贴合
        el.scrollTop += drift;
        deps.onProgrammaticScroll(el.scrollTop);
      }
      rounds++;
      if (nowMs() - startedAt >= NAV_SETTLE_MS) {
        onEnd(false, drift, rounds);
        return;
      }
      navAnimRaf = requestAnimationFrame(step);
    };
    navAnimRaf = requestAnimationFrame(step);
  }

  /** 跳转收尾：同步索引 → 登记落点 → 高亮目标 → 延后一帧关窗 */
  function finishJump(
    el: HTMLElement,
    node: HTMLElement,
    entry: TurnNavTarget,
    seq: number,
    info: { landedDrift: number; drift: number; rounds: number; settled: boolean },
  ) {
    if (seq !== navAnimSeq) return;
    syncAnchors();
    deps.onProgrammaticScroll(el.scrollTop);
    if (el.contains(node)) flashTurn(node);
    detachJumpGuards();
    // 只在落点明显漂移或未收敛时留诊断日志（同事机器回传即可定位成因）
    if (!info.settled || Math.abs(info.landedDrift) > NAV_DRIFT_LOG_PX) {
      logJump(
        "chat-nav-jump",
        `target=${entry.index + 1} drift=${Math.round(info.landedDrift)}px ` +
          `residual=${Math.round(info.drift)}px rounds=${info.rounds} ` +
          `settled=${info.settled ? 1 : 0}`,
      );
    }
    // 跳转窗口延后一帧关闭：scroll 事件在每帧的滚动步骤里派发（早于 rAF 回调），
    // 结束帧写入引发的事件要到下一帧才到，提前关窗会被「距底 60px 内」误判成
    // 用户滚动而重新开启吸底，把视图从落点拉回底部
    navAnimRaf = requestAnimationFrame(() => {
      if (seq !== navAnimSeq) return;
      navAnimRaf = undefined;
      navAnimating = false;
    });
  }

  /**
   * 滚到指定用户消息回合起点（顶部对齐）：量取 → 自绘动画 → 跟随收敛 → 收尾高亮。
   * 跳转窗口从点击起（含 measuring/nextTick 等待）一直开到最后一次跟随结束的下一帧，
   * 期间的 scroll 事件都视为程序化滚动——流式吸底写入留下的滞后事件恰落在这个窗口内，
   * 否则会被「距底 60px 内」判据当成用户滚动而重新开启吸底。
   */
  async function jumpToTurn(entry: TurnNavTarget) {
    const el = deps.scroller.value;
    if (!el) return;
    if (entry.index < 0 || entry.index >= anchorCount.value) {
      logJump(
        "chat-nav-jump",
        `target=${entry.index + 1}/${anchorCount.value} early=range`,
      );
      return;
    }
    // 手动浏览历史：解除吸底，避免 MutationObserver/流式更新把位置拉回
    deps.stickToBottom.value = false;
    cancelTurnAnimation();
    const seq = navAnimSeq;
    navAnimating = true;
    currentIndex.value = entry.index;
    attachJumpGuards(el);
    const measured = await measureAnchor(el, entry, seq);
    if (measured === "cancelled") return;
    if (!measured) {
      logJump("chat-nav-jump", `target=${entry.index + 1} early=missing`);
      endJumpWindow(seq);
      detachJumpGuards();
      return;
    }
    if (!measured.byId) {
      logJump(
        "chat-nav-anchor-fallback",
        `target=${entry.index + 1} id=${entry.id}`,
      );
    }
    await nextTick();
    if (seq !== navAnimSeq) return;
    animateScrollTo(el, measured.top, seq, () => {
      if (seq !== navAnimSeq) return;
      const landedDrift = el.contains(measured.node)
        ? anchorDrift(el, measured.node)
        : 0;
      followAnchorToTop(el, measured.node, seq, (settled, drift, rounds) => {
        finishJump(el, measured.node, entry, seq, {
          landedDrift,
          drift,
          rounds,
          settled,
        });
      });
    });
  }

  /** 可信用户滚动时更新当前回合索引（由吸底滚动引擎回调） */
  function onTrustedScroll(scrollTop: number) {
    currentIndex.value = findCurrentTurnIndex(anchorTops, scrollTop);
  }

  /** 切换线程/重新加载时复位导航与预热状态 */
  function resetTurnNav() {
    resetWarmState();
    cancelTurnAnimation();
    detachJumpGuards();
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
    detachJumpGuards();
    clearWarmRetry();
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
