import { defineComponent, ref } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { useCtrlWheelZoom } from "../useCtrlWheelZoom";

const Harness = defineComponent({
  setup() {
    const container = ref<HTMLElement | null>(null);
    const zoom = ref(1);
    useCtrlWheelZoom(container, zoom, 0.5, 4, 1.25);
    return { container, zoom };
  },
  template: `<div ref="container" class="zone"></div>`,
});

function wheel(el: Element, deltaY: number, ctrlKey = true): WheelEvent {
  const e = new WheelEvent("wheel", { deltaY, bubbles: true, cancelable: true });
  // happy-dom 的 WheelEvent 构造不接收 ctrlKey，手动定义
  Object.defineProperty(e, "ctrlKey", { value: ctrlKey, configurable: true });
  el.dispatchEvent(e);
  return e;
}

describe("useCtrlWheelZoom", () => {
  it("ctrl+滚轮上滚放大、下滚缩小，并 preventDefault", async () => {
    const w = mount(Harness);
    await flushPromises();
    const el = w.find(".zone").element;
    const vm = w.vm as unknown as { zoom: number };

    const up = wheel(el, -100);
    expect(up.defaultPrevented).toBe(true);
    expect(vm.zoom).toBe(1.25);

    const down = wheel(el, 100);
    expect(down.defaultPrevented).toBe(true);
    expect(vm.zoom).toBe(1);
  });

  it("非 ctrl 滚轮不拦截不缩放", async () => {
    const w = mount(Harness);
    await flushPromises();
    const el = w.find(".zone").element;
    const vm = w.vm as unknown as { zoom: number };

    const e = wheel(el, -100, false);
    expect(e.defaultPrevented).toBe(false);
    expect(vm.zoom).toBe(1);
  });

  it("缩放在上下限内夹紧", async () => {
    const w = mount(Harness);
    await flushPromises();
    const el = w.find(".zone").element;
    const vm = w.vm as unknown as { zoom: number };

    for (let i = 0; i < 20; i++) wheel(el, -100);
    expect(vm.zoom).toBe(4);
    for (let i = 0; i < 20; i++) wheel(el, 100);
    expect(vm.zoom).toBe(0.5);
  });
});
