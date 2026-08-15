import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useComposerResize } from "../useComposerResize";

function pointerEvent(clientY: number): PointerEvent {
  return new MouseEvent("pointermove", { clientY }) as unknown as PointerEvent;
}

describe("useComposerResize", () => {
  beforeEach(() => {
    Object.defineProperty(window, "innerHeight", {
      value: 600,
      configurable: true,
    });
  });
  afterEach(() => {
    document.body.classList.remove("resizing-composer");
  });

  it("start → pointermove 更新高度（钳制 120..半窗），pointerup 清理", () => {
    const r = useComposerResize();
    const start = {
      preventDefault: vi.fn(),
      clientY: 300,
    } as unknown as PointerEvent;
    r.startComposerResize(start);
    expect(r.resizingComposer.value).toBe(true);
    expect(document.body.classList.contains("resizing-composer")).toBe(true);

    window.dispatchEvent(pointerEvent(200)); // 上移 100 → 高度 220
    expect(r.composerHeight.value).toBe(220);

    window.dispatchEvent(pointerEvent(-2000)); // 大幅上移 → 钳制半窗 300
    expect(r.composerHeight.value).toBe(300);

    window.dispatchEvent(pointerEvent(2000)); // 大幅下移 → 钳制最低 120
    expect(r.composerHeight.value).toBe(120);

    window.dispatchEvent(
      new MouseEvent("pointerup") as unknown as PointerEvent,
    );
    expect(r.resizingComposer.value).toBe(false);
    expect(document.body.classList.contains("resizing-composer")).toBe(false);
  });

  it("clampComposerHeightOnResize 收缩超出上限的高度", () => {
    const r = useComposerResize();
    r.composerHeight.value = 500;
    r.clampComposerHeightOnResize();
    expect(r.composerHeight.value).toBe(300);
  });
});
