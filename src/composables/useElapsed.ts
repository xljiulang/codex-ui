import { computed, ref } from "vue";
import { useClock } from "./useClock";

/**
 * 实时计时：基于全局共享时钟（250ms 一跳），返回毫秒数。
 * 组件卸载后自动取消对共享时钟的订阅，不再持有独立定时器。
 */
export function useElapsed(startAtMs: number) {
  const started = ref(startAtMs);
  const now = useClock();
  const elapsed = computed(() => Math.max(0, now.value - started.value));
  return { elapsed };
}
