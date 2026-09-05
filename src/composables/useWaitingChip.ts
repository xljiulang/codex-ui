import { computed, onBeforeUnmount, ref, watch, type ComputedRef } from "vue";

/** 等待响应提示的显示前提（由调用方按会话标签计算并注入） */
export interface WaitingChipDeps {
  /** 回合进行中（tab.turnActive） */
  turnActive: ComputedRef<boolean>;
  /** 有正在流式输出或进行中的工具/命令（useCodex 按线程增量维护） */
  hasActiveWork: ComputedRef<boolean>;
  /** 当前线程消息数（无消息时不显示） */
  itemCount: ComputedRef<number>;
  /** 待处理交互数（有交互挂起时不显示） */
  interactionCount: ComputedRef<number>;
}

/**
 * 等待响应提示：chip 显示期间本地计时并高频刷新秒数（100ms 步进）。
 */
export function useWaitingChip(deps: WaitingChipDeps) {
  const showWaiting = computed(
    () =>
      deps.turnActive.value &&
      !deps.hasActiveWork.value &&
      deps.itemCount.value > 0 &&
      deps.interactionCount.value === 0,
  );
  const waitingElapsedMs = ref(0);
  let waitingStartedAt = 0;
  let waitingTimer: number | undefined;
  function stopWaitingTimer() {
    if (waitingTimer !== undefined) {
      window.clearInterval(waitingTimer);
      waitingTimer = undefined;
    }
  }
  function startWaitingTimer() {
    waitingStartedAt = Date.now();
    waitingElapsedMs.value = 0;
    stopWaitingTimer();
    waitingTimer = window.setInterval(() => {
      waitingElapsedMs.value = Date.now() - waitingStartedAt;
    }, 100);
  }
  watch(
    showWaiting,
    (v) => {
      if (v) startWaitingTimer();
      else {
        stopWaitingTimer();
        waitingElapsedMs.value = 0;
      }
    },
    { immediate: true },
  );
  const waitingSeconds = computed(() =>
    (waitingElapsedMs.value / 1000).toFixed(1),
  );
  onBeforeUnmount(stopWaitingTimer);
  return { showWaiting, waitingSeconds, stopWaitingTimer };
}
