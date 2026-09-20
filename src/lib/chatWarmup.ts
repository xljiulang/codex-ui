/**
 * 会话历史全文预热：首次跳转/吸底落定前强制真实渲染一次，
 * 使 content-visibility 记住全部真实高度（约 80px 估算高度的兜底）。
 */

import { pendingMarkdownCount } from "./markdownRenderer";

// 首次跳转前的全文预热参数：渲染安静期与整体硬上限
// 硬上限取较大值：大历史会话的正文落地（Worker 渲染 + 代码高亮）本身就可能耗时数秒，
// 上限只用于兜底「永远不会安静」的异常场景，超时会明确标记为「未稳定」而不是当成稳定。
export const WARM_QUIET_MS = 120;
export const WARM_MAX_MS = 8000;

/**
 * 等待 Markdown Worker / 代码块装饰等异步 DOM 落地：
 * 连续 WARM_QUIET_MS 无子树变化 **且** 待渲染 Markdown 队列已清空才算稳定。
 * 返回 stable=false 表示等待到 WARM_MAX_MS 上限仍未稳定（调用方不应据此认定布局已定）。
 */
export function waitForRenderQuiet(el: HTMLElement): Promise<{ stable: boolean }> {
  return new Promise((resolve) => {
    let observer: MutationObserver | undefined;
    let settled = false;
    let quietTimer: number | undefined;
    let hardTimer: number | undefined;
    const finish = (stable: boolean) => {
      if (settled) return;
      settled = true;
      if (quietTimer !== undefined) window.clearTimeout(quietTimer);
      if (hardTimer !== undefined) window.clearTimeout(hardTimer);
      observer?.disconnect();
      resolve({ stable });
    };
    // 安静期到点：正文仍有待落地渲染时继续等（把「还在长高」排除在稳定之外）
    const armQuiet = () => {
      if (quietTimer !== undefined) window.clearTimeout(quietTimer);
      quietTimer = window.setTimeout(() => {
        quietTimer = undefined;
        if (pendingMarkdownCount() > 0) {
          armQuiet();
          return;
        }
        finish(true);
      }, WARM_QUIET_MS);
    };
    if (typeof MutationObserver === "undefined") {
      // 无观察能力：只按渲染队列判定，等一个安静期即收尾
      armQuiet();
    } else {
      observer = new MutationObserver(() => armQuiet());
      observer.observe(el, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ["class", "style"],
      });
      armQuiet();
    }
    hardTimer = window.setTimeout(() => finish(false), WARM_MAX_MS);
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
