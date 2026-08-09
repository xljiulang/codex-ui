import { ref, watch, type Ref } from "vue";

export interface ThrottledRef<T> {
  ref: Ref<T>;
  /** 立即把目标值刷新为源的最新值（流式结束时调用） */
  flush: () => void;
}

/**
 * 返回一个最多每 intervalMs 更新一次的 Ref。
 * 源连续变化时只记录最新 pending，定时器触发时一次性写入目标，
 * 避免流式场景下每个 delta 都触发整段 DOM 重渲染。
 */
export function useThrottledRef<T>(
  source: Ref<T>,
  intervalMs = 80,
): ThrottledRef<T> {
  const target = ref(source.value) as Ref<T>;
  let pending: T | undefined;
  let timer: number | undefined;

  function flush() {
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timer = undefined;
    }
    pending = undefined;
    target.value = source.value;
  }

  watch(
    source,
    (v) => {
      pending = v;
      if (timer === undefined) {
        timer = window.setTimeout(() => {
          timer = undefined;
          target.value = pending as T;
          pending = undefined;
        }, intervalMs);
      }
    },
    // 同步触发：源变化（如流式 delta 写入）时立即登记 pending 并安排定时器，
    // 否则异步 watch 会把节流起点推迟到微任务之后，测试与行为都不可预期。
    { flush: "sync" },
  );

  return { ref: target, flush };
}
