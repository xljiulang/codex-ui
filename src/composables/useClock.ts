import { onBeforeUnmount, ref } from "vue";

const TICK_MS = 250;

const now = ref(Date.now());
let timer: number | undefined;
let subscribers = 0;

function ensureTicking() {
  if (timer === undefined) {
    timer = window.setInterval(() => {
      now.value = Date.now();
    }, TICK_MS);
  }
}

function release() {
  subscribers--;
  if (subscribers <= 0 && timer !== undefined) {
    window.clearInterval(timer);
    timer = undefined;
    subscribers = 0;
  }
}

/**
 * 全局共享时钟：整个应用只有一个 250ms interval。
 * 首个订阅者启动，最后一个卸载后自动停止；返回的 ref 对所有组件一致。
 */
export function useClock() {
  subscribers++;
  ensureTicking();
  onBeforeUnmount(release);
  return now;
}
