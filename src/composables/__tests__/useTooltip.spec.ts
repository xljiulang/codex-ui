import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hideTooltip, showTooltip, tooltip } from "../useTooltip";

function resetTooltipState() {
  tooltip.visible = false;
  tooltip.text = "";
  tooltip.anchor = null;
  tooltip.anchorEl = null;
}

describe("useTooltip 全局 tooltip 状态", () => {
  beforeEach(() => {
    resetTooltipState();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
    resetTooltipState();
  });

  it("showTooltip 设置文本、锚点并显示", () => {
    const rect = { left: 10, top: 20, width: 30, height: 40 } as DOMRect;
    showTooltip("提示内容", rect);
    expect(tooltip.visible).toBe(true);
    expect(tooltip.text).toBe("提示内容");
    expect(tooltip.anchor).toEqual({
      left: 10,
      top: 20,
      width: 30,
      height: 40,
    });
    expect(tooltip.anchorEl).toBeNull();
  });

  it("空白文本忽略，不显示", () => {
    const rect = { left: 0, top: 0, width: 0, height: 0 } as DOMRect;
    showTooltip("   ", rect);
    expect(tooltip.visible).toBe(false);
    expect(tooltip.text).toBe("");
  });

  it("hideTooltip 延迟 60ms 后隐藏并清空锚点", () => {
    const rect = { left: 0, top: 0, width: 10, height: 10 } as DOMRect;
    showTooltip("x", rect);
    hideTooltip();
    expect(tooltip.visible).toBe(true);
    vi.advanceTimersByTime(60);
    expect(tooltip.visible).toBe(false);
    expect(tooltip.anchor).toBeNull();
  });

  it("锚点元素被隐藏/指针移出实时包围盒时兜底隐藏", () => {
    const el = document.createElement("button");
    el.style.display = "block";
    document.body.appendChild(el);
    vi.spyOn(el, "getBoundingClientRect").mockReturnValue({
      left: 0,
      top: 0,
      right: 100,
      bottom: 50,
      width: 100,
      height: 50,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect);
    const rect = { left: 0, top: 0, width: 10, height: 10 } as DOMRect;
    showTooltip("锚点提示", rect, el);
    expect(tooltip.visible).toBe(true);
    window.dispatchEvent(
      new MouseEvent("mousemove", { clientX: 500, clientY: 500 }),
    );
    vi.advanceTimersByTime(60);
    expect(tooltip.visible).toBe(false);
    expect(tooltip.anchorEl).toBeNull();
  });
});
