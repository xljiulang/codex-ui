import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { mount } from "@vue/test-utils";
import {
  hideTooltip,
  showTooltip,
  tooltip,
} from "../../composables/useTooltip";
import { tooltipDirective } from "../tooltip";

const Host = {
  template: `<button v-tooltip="tip">按钮</button>`,
  data: () => ({ tip: "提示" }),
};

function attachEl(): HTMLButtonElement {
  const el = document.createElement("button");
  el.textContent = "按钮";
  document.body.appendChild(el);
  return el;
}

describe("useTooltip 显隐与指针守卫", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    hideTooltip();
    vi.advanceTimersByTime(100);
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("showTooltip 记录文本/锚点/归属元素并显示", () => {
    const el = attachEl();
    showTooltip("提示", el.getBoundingClientRect(), el);
    expect(tooltip.visible).toBe(true);
    expect(tooltip.text).toBe("提示");
    expect(tooltip.anchorEl).toBe(el);
    expect(tooltip.anchor).toEqual({
      left: 0,
      top: 0,
      width: 0,
      height: 0,
    });
  });

  it("空文本不显示 tooltip", () => {
    showTooltip("   ", attachEl().getBoundingClientRect());
    expect(tooltip.visible).toBe(false);
  });

  it("hideTooltip 延迟 60ms 后隐藏并清理归属", () => {
    const el = attachEl();
    showTooltip("x", el.getBoundingClientRect(), el);
    hideTooltip();
    expect(tooltip.visible).toBe(true);
    vi.advanceTimersByTime(60);
    expect(tooltip.visible).toBe(false);
    expect(tooltip.anchorEl).toBeNull();
    expect(tooltip.anchor).toBeNull();
  });

  it("不传归属元素（MarkdownText 旧式调用）行为不变", () => {
    showTooltip("x", attachEl().getBoundingClientRect());
    expect(tooltip.visible).toBe(true);
    expect(tooltip.anchorEl).toBeNull();
    hideTooltip();
    vi.advanceTimersByTime(60);
    expect(tooltip.visible).toBe(false);
  });

  it("指针守卫：仍在锚点包围盒内不隐藏，移出后隐藏", () => {
    const el = attachEl();
    // happy-dom 下 getBoundingClientRect 为全 0：(0,0) 视为在包围盒内
    showTooltip("x", el.getBoundingClientRect(), el);
    window.dispatchEvent(
      new MouseEvent("mousemove", { clientX: 0, clientY: 0 }),
    );
    expect(tooltip.visible).toBe(true);
    window.dispatchEvent(
      new MouseEvent("mousemove", { clientX: 100, clientY: 100 }),
    );
    vi.advanceTimersByTime(60);
    expect(tooltip.visible).toBe(false);
    expect(tooltip.anchorEl).toBeNull();
  });

  it("指针守卫：锚点元素 display:none 后下一次 mousemove 隐藏", () => {
    const el = attachEl();
    showTooltip("x", el.getBoundingClientRect(), el);
    el.style.display = "none";
    window.dispatchEvent(
      new MouseEvent("mousemove", { clientX: 0, clientY: 0 }),
    );
    vi.advanceTimersByTime(60);
    expect(tooltip.visible).toBe(false);
  });
});

describe("tooltip 指令", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    hideTooltip();
    vi.advanceTimersByTime(100);
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("mouseenter 显示并记录归属元素，卸载时隐藏", async () => {
    const wrapper = mount(Host, {
      global: { directives: { tooltip: tooltipDirective } },
      attachTo: document.body,
    });
    const btn = wrapper.find("button");
    await btn.trigger("mouseenter");
    expect(tooltip.visible).toBe(true);
    expect(tooltip.anchorEl).toBe(btn.element);

    wrapper.unmount();
    vi.advanceTimersByTime(60);
    expect(tooltip.visible).toBe(false);
  });

  it("mouseleave 隐藏 tooltip", async () => {
    const wrapper = mount(Host, {
      global: { directives: { tooltip: tooltipDirective } },
      attachTo: document.body,
    });
    await wrapper.find("button").trigger("mouseenter");
    expect(tooltip.visible).toBe(true);
    await wrapper.find("button").trigger("mouseleave");
    vi.advanceTimersByTime(60);
    expect(tooltip.visible).toBe(false);
    wrapper.unmount();
  });

  it("归属元素被 style 隐藏（MutationObserver）时收起", async () => {
    const wrapper = mount(Host, {
      global: { directives: { tooltip: tooltipDirective } },
      attachTo: document.body,
    });
    const btn = wrapper.find("button");
    await btn.trigger("mouseenter");
    expect(tooltip.visible).toBe(true);

    btn.element.style.display = "none";
    await nextTick();
    await Promise.resolve();
    vi.advanceTimersByTime(60);
    expect(tooltip.visible).toBe(false);
    wrapper.unmount();
  });

  it("无关元素卸载不影响当前 tooltip", async () => {
    const wrapperA = mount(Host, {
      global: { directives: { tooltip: tooltipDirective } },
      attachTo: document.body,
    });
    const wrapperB = mount(Host, {
      global: { directives: { tooltip: tooltipDirective } },
      attachTo: document.body,
    });
    await wrapperA.find("button").trigger("mouseenter");
    expect(tooltip.anchorEl).toBe(wrapperA.find("button").element);

    wrapperB.unmount();
    vi.advanceTimersByTime(100);
    expect(tooltip.visible).toBe(true);

    wrapperA.unmount();
    vi.advanceTimersByTime(60);
    expect(tooltip.visible).toBe(false);
  });
});
