import { describe, expect, it, vi } from "vitest";
import { defineComponent, h } from "vue";
import { mount } from "@vue/test-utils";
import { useClock } from "../useClock";

const ClockProbe = defineComponent({
  setup() {
    useClock();
    return () => h("div");
  },
});

describe("useClock 全局共享时钟", () => {
  it("多个订阅者共享一个 interval，最后一个卸载后停止", () => {
    const setSpy = vi.spyOn(window, "setInterval");
    const clearSpy = vi.spyOn(window, "clearInterval");
    const a = mount(ClockProbe);
    const b = mount(ClockProbe);
    expect(setSpy).toHaveBeenCalledTimes(1);
    a.unmount();
    expect(clearSpy).not.toHaveBeenCalled();
    b.unmount();
    expect(clearSpy).toHaveBeenCalledTimes(1);
    setSpy.mockRestore();
    clearSpy.mockRestore();
  });

  it("全部卸载后重新订阅会再次启动 interval", () => {
    const setSpy = vi.spyOn(window, "setInterval");
    const clearSpy = vi.spyOn(window, "clearInterval");
    const a = mount(ClockProbe);
    a.unmount();
    expect(clearSpy).toHaveBeenCalledTimes(1);
    const b = mount(ClockProbe);
    expect(setSpy).toHaveBeenCalledTimes(2);
    b.unmount();
    setSpy.mockRestore();
    clearSpy.mockRestore();
  });
});
