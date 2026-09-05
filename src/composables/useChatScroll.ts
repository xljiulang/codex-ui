import { nextTick, ref, type Ref } from "vue";

/** 吸底滚动引擎与回合导航域的接缝（由 ChatView 注入，避免双向依赖） */
export interface ChatScrollDeps {
  /** 回合跳转自绘动画进行中：期间滚动事件不参与吸底/索引推导 */
  isNavAnimating(): boolean;
  /** 可信用户滚动：更新当前回合索引（锚点归属导航域） */
  onTrustedScroll(scrollTop: number): void;
  /** 内容/位置变化后重算回合锚点 */
  scheduleAnchorSync(): void;
  /** 吸底落定后补一次后台全文预热 */
  onSettled(): void;
}

/** 解除吸底所需的最小距底距离：轻微误触/抖动（< 32px）不解除，
 *  单次正常滚轮（约 80-100px）一次即解除 */
const UNPIN_DIST_PX = 32;

/**
 * 聊天消息流吸底滚动引擎：
 * - 吸底跟随（用户上滑看历史时暂停，回到底部附近自动恢复）；
 * - 历史会话整批落地后的强制测量吸底（settleToBottom）；
 * - DOM 结构变化兜底（MutationObserver）与图片懒加载跟随。
 */
export function useChatScroll(
  scroller: Ref<HTMLElement | null>,
  deps: ChatScrollDeps,
) {
  // 是否吸附在底部：用户上滑查看历史时暂停自动滚动
  const stickToBottom = ref(true);
  // 最近一次程序化吸底写入/滚动事件后的 scrollTop：
  // 用户可信滚动相对该位置向上移动即视为“上滑看历史”，一次正常上滑立即解除；
  // 向下（追赶窗口内）或程序化滚动不会误解除。
  let lastStickScrollTop = 0;
  // settleToBottom 进行中标记（供预热守卫复用）
  let settleRunning = false;
  let scrollRaf: number | undefined;
  let scrollObserver: MutationObserver | undefined;

  function onScroll(e: Event) {
    // 自绘跳转动画期间的滚动事件不参与索引/吸底推导，结束后由 syncAnchors 收敛
    if (deps.isNavAnimating()) return;
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
      deps.onTrustedScroll(el.scrollTop);
    }
  }

  // 吸底滚动合并到每帧一次：流式高频变更时避免每次都强制整块布局
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
    deps.scheduleAnchorSync();
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
    deps.scheduleAnchorSync();
    deps.onSettled();
  }

  /** 切换会话后重置吸底状态，避免旧会话的上滑状态残留导致新对话默认不吸底 */
  function resetForNewThread() {
    stickToBottom.value = true;
    lastStickScrollTop = 0;
    scheduleScroll();
    deps.scheduleAnchorSync();
  }

  // 懒加载/异步图片加载后高度变化没有 DOM 结构变更，补一次跟随
  function onImageLoad(e: Event) {
    if ((e.target as Element | null)?.tagName === "IMG") {
      scheduleScroll();
      deps.scheduleAnchorSync();
    }
  }

  /** 挂载后建立 DOM 兜底观察：结构变化立即吸底（不等下一帧），高频变更以 rAF 合并 */
  function attachObserver() {
    stickToBottom.value = true;
    if (scroller.value && typeof MutationObserver !== "undefined") {
      // worker 渲染 markdown / 命令输出 HTML 落地晚于数据变更
      scrollObserver = new MutationObserver(() => {
        if (stickToBottom.value && scroller.value) {
          scroller.value.scrollTop = scroller.value.scrollHeight;
          lastStickScrollTop = scroller.value.scrollTop;
        }
        scheduleScroll();
        deps.scheduleAnchorSync();
      });
      scrollObserver.observe(scroller.value, { childList: true, subtree: true });
      scroller.value.addEventListener("load", onImageLoad, true);
      lastStickScrollTop = scroller.value.scrollTop;
    }
    scheduleScroll();
  }

  function dispose() {
    if (scrollRaf !== undefined) cancelAnimationFrame(scrollRaf);
    scrollRaf = undefined;
    scrollObserver?.disconnect();
    scrollObserver = undefined;
    scroller.value?.removeEventListener("load", onImageLoad, true);
    // 避免切换视图后残留滚动状态
    stickToBottom.value = true;
  }

  return {
    stickToBottom,
    isSettling: () => settleRunning,
    onScroll,
    scheduleScroll,
    jumpToBottom,
    settleToBottom,
    resetForNewThread,
    attachObserver,
    dispose,
  };
}
