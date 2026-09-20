import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// 预热稳定性依赖「待渲染 Markdown 队列」，这里把它替换成可注入的计数
const state = vi.hoisted(() => ({ pending: 0 }));
vi.mock("../markdownRenderer", () => ({
  pendingMarkdownCount: () => state.pending,
}));

import {
  WARM_MAX_MS,
  WARM_QUIET_MS,
  loadPendingImages,
  waitForRenderQuiet,
} from "../chatWarmup";

describe("waitForRenderQuiet（预热稳定判定）", () => {
  let el: HTMLElement;

  beforeEach(() => {
    vi.useFakeTimers();
    state.pending = 0;
    el = document.createElement("div");
    document.body.appendChild(el);
  });

  afterEach(() => {
    el.remove();
    vi.useRealTimers();
  });

  it("DOM 安静且渲染队列清空 → 判定为稳定", async () => {
    const p = waitForRenderQuiet(el);
    await vi.advanceTimersByTimeAsync(WARM_QUIET_MS + 20);
    await expect(p).resolves.toEqual({ stable: true });
  });

  it("渲染队列未清空时不算稳定：清空后仍需再等一个安静期", async () => {
    state.pending = 3;
    const p = waitForRenderQuiet(el);
    let done = false;
    void p.then(() => {
      done = true;
    });
    await vi.advanceTimersByTimeAsync(300);
    expect(done).toBe(false);

    state.pending = 0;
    await vi.advanceTimersByTimeAsync(WARM_QUIET_MS + 20);
    await expect(p).resolves.toEqual({ stable: true });
  });

  it("持续 DOM 变更到上限：返回未稳定，而不是把未稳定当稳定", async () => {
    const p = waitForRenderQuiet(el);
    for (let i = 0; i < Math.ceil(WARM_MAX_MS / 100) + 2; i++) {
      el.appendChild(document.createElement("span"));
      await vi.advanceTimersByTimeAsync(100);
    }
    await expect(p).resolves.toEqual({ stable: false });
  });

  it("每个变更都重置安静期：变更停止后才收尾", async () => {
    const p = waitForRenderQuiet(el);
    let done = false;
    void p.then(() => {
      done = true;
    });
    for (let i = 0; i < 20; i++) {
      el.appendChild(document.createElement("span"));
      await vi.advanceTimersByTimeAsync(100);
    }
    expect(done).toBe(false);

    await vi.advanceTimersByTimeAsync(WARM_QUIET_MS + 20);
    await expect(p).resolves.toEqual({ stable: true });
  });

  it("无 MutationObserver 时只按渲染队列判定，仍能收尾", async () => {
    vi.stubGlobal("MutationObserver", undefined);
    const p = waitForRenderQuiet(el);
    await vi.advanceTimersByTimeAsync(WARM_QUIET_MS + 20);
    await expect(p).resolves.toEqual({ stable: true });
    vi.unstubAllGlobals();
  });

  it("懒加载图片临时置 eager 等解码，完成后恢复 lazy；已完成的图片不处理", async () => {
    const img = document.createElement("img");
    img.loading = "lazy";
    Object.defineProperty(img, "complete", { configurable: true, value: false });
    const decode = vi.fn(() => Promise.resolve());
    Object.defineProperty(img, "decode", { configurable: true, value: decode });
    const doneImg = document.createElement("img");
    Object.defineProperty(doneImg, "complete", {
      configurable: true,
      value: true,
    });
    el.append(img, doneImg);

    const p = loadPendingImages(el);
    expect(img.loading).toBe("eager");
    await vi.advanceTimersByTimeAsync(0);
    await p;
    expect(decode).toHaveBeenCalledTimes(1);
    expect(img.loading).toBe("lazy");
  });

  it("没有待加载图片时立即返回", async () => {
    await expect(loadPendingImages(el)).resolves.toBeUndefined();
  });
});
