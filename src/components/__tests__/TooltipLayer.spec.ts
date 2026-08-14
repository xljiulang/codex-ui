import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import TooltipLayer from "../TooltipLayer.vue";
import { tooltip } from "../../composables/useTooltip";

describe("TooltipLayer 全局 tooltip 渲染", () => {
  it("tooltip 可见时渲染文本与 role", async () => {
    tooltip.visible = false;
    const w = mount(TooltipLayer);
    expect(w.find(".app-tooltip").exists()).toBe(false);
    tooltip.visible = true;
    tooltip.text = "提示文本";
    await w.vm.$nextTick();
    const el = w.find(".app-tooltip");
    expect(el.exists()).toBe(true);
    expect(el.attributes("role")).toBe("tooltip");
    expect(el.text()).toBe("提示文本");
  });

  it("隐藏后不渲染", async () => {
    tooltip.visible = true;
    tooltip.text = "x";
    const w = mount(TooltipLayer);
    await w.vm.$nextTick();
    expect(w.find(".app-tooltip").exists()).toBe(true);
    tooltip.visible = false;
    await w.vm.$nextTick();
    expect(w.find(".app-tooltip").exists()).toBe(false);
  });
});
