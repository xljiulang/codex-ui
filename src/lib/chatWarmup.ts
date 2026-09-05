/**
 * 会话历史全文预热：首次跳转/吸底落定前强制真实渲染一次，
 * 使 content-visibility 记住全部真实高度（约 80px 估算高度的兜底）。
 */

// 首次跳转前的全文预热参数：渲染安静期与整体硬上限
export const WARM_QUIET_MS = 120;
export const WARM_MAX_MS = 2000;

/**
 * 等待 Markdown Worker / 代码块装饰等异步 DOM 落地：
 * 连续 WARM_QUIET_MS 无子树变化即认为稳定，WARM_MAX_MS 硬上限兜底。
 */
export function waitForRenderQuiet(el: HTMLElement): Promise<void> {
  return new Promise((resolve) => {
    if (typeof MutationObserver === "undefined") {
      window.setTimeout(resolve, WARM_QUIET_MS);
      return;
    }
    let settled = false;
    let quietTimer: number | undefined;
    let hardTimer: number | undefined;
    const finish = () => {
      if (settled) return;
      settled = true;
      if (quietTimer !== undefined) window.clearTimeout(quietTimer);
      if (hardTimer !== undefined) window.clearTimeout(hardTimer);
      observer.disconnect();
      resolve();
    };
    const kickQuiet = () => {
      if (quietTimer !== undefined) window.clearTimeout(quietTimer);
      quietTimer = window.setTimeout(finish, WARM_QUIET_MS);
    };
    const observer = new MutationObserver(kickQuiet);
    observer.observe(el, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "style"],
    });
    kickQuiet();
    hardTimer = window.setTimeout(finish, WARM_MAX_MS);
  });
}

/** 临时把仍为懒加载的图片置为 eager 并等待解码，完成后恢复 lazy */
export async function loadPendingImages(el: HTMLElement): Promise<void> {
  const pending = Array.from(
    el.querySelectorAll<HTMLImageElement>("img"),
  ).filter((img) => !img.complete);
  if (pending.length === 0) return;
  const eager: HTMLImageElement[] = [];
  for (const img of pending) {
    if (img.loading === "lazy") {
      eager.push(img);
      img.loading = "eager";
    }
  }
  try {
    await Promise.race([
      Promise.allSettled(
        pending.map((img) =>
          img.complete
            ? Promise.resolve()
            : typeof img.decode === "function"
              ? img.decode().catch(() => undefined)
              : Promise.resolve(),
        ),
      ),
      new Promise<void>((resolve) => {
        window.setTimeout(resolve, WARM_MAX_MS);
      }),
    ]);
  } finally {
    for (const img of eager) img.loading = "lazy";
  }
}
