import { onBeforeUnmount, ref } from "vue";

/**
 * 实时计时器：从 startAtMs 开始每 250ms 刷新，返回 mm:ss.s 格式。
 * 组件卸载时自动清理。
 */
export function useElapsed(startAtMs: number) {
  const elapsed = ref(0);
  const started = ref(startAtMs);

  const tick = () => {
    elapsed.value = Date.now() - started.value;
  };

  const timer = setInterval(tick, 250);
  tick();

  onBeforeUnmount(() => clearInterval(timer));

  return { elapsed };
}
