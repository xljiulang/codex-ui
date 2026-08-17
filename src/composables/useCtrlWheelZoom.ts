import { onBeforeUnmount, watch, type Ref } from "vue";

/**
 * Ctrl + 滚轮缩放：在内容容器上挂原生非 passive wheel 监听，
 * 拦截浏览器/WebView2 默认的整页缩放，按步进调整预览 zoom。
 * 上滚（deltaY < 0）放大、下滚缩小，夹在 [min, max]。
 * Vue 模板 @wheel 默认 passive 无法 preventDefault，必须用原生监听。
 */
export function useCtrlWheelZoom(
  container: Ref<HTMLElement | null>,
  zoom: Ref<number>,
  min: number,
  max: number,
  step = 1.25,
) {
  function onWheel(e: WheelEvent) {
    if (!e.ctrlKey) return;
    e.preventDefault();
    zoom.value = Math.min(
      max,
      Math.max(min, zoom.value * (e.deltaY < 0 ? step : 1 / step)),
    );
  }

  watch(
    container,
    (el, old) => {
      old?.removeEventListener("wheel", onWheel);
      el?.addEventListener("wheel", onWheel, { passive: false });
    },
    { flush: "post" },
  );
  onBeforeUnmount(() => {
    container.value?.removeEventListener("wheel", onWheel);
  });
}
