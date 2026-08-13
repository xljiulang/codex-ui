/** 统一防抖工具：run 触发（自动取消上一次），cancel 手动取消未执行的调用 */
export function debounce<A extends unknown[]>(
  fn: (...args: A) => void,
  delay: number,
): { run: (...args: A) => void; cancel: () => void } {
  let timer: number | undefined;
  const run = (...args: A) => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => fn(...args), delay);
  };
  const cancel = () => {
    window.clearTimeout(timer);
    timer = undefined;
  };
  return { run, cancel };
}
