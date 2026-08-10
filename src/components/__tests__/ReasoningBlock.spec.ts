import { describe, expect, it } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ReasoningBlock from "../ReasoningBlock.vue";
import type { ThreadItem } from "../../lib/types";

function reasoningItem(lines: number, streaming = true): ThreadItem {
  const content = Array.from({ length: lines }, (_, i) => `第 ${i + 1} 行思考内容`);
  return {
    id: "r1",
    type: "reasoning",
    content,
    streaming,
    startedAtMs: 1000,
  } as ThreadItem;
}

describe("ReasoningBlock 完整展示", () => {
  it("长内容完整渲染，不再限高、无展开按钮", async () => {
    const wrapper = mount(ReasoningBlock, {
      props: { item: reasoningItem(20) },
    });
    await flushPromises();
    // 默认折叠，点击后展开
    expect(wrapper.find(".reasoning-toggle").attributes("aria-expanded")).toBe(
      "false",
    );
    await wrapper.find(".reasoning-toggle").trigger("click");
    const content = wrapper.find(".reasoning-content");
    expect(content.classes()).not.toContain("capped");
    expect(wrapper.find(".reasoning-expand").exists()).toBe(false);
    expect(content.text()).toContain("第 20 行思考内容");
  });

  it("aria-expanded 随开合切换", async () => {
    const wrapper = mount(ReasoningBlock, {
      props: { item: reasoningItem(3, false) },
    });
    await flushPromises();
    // 非流式时默认折叠
    expect(wrapper.find(".reasoning-toggle").attributes("aria-expanded")).toBe(
      "false",
    );
    await wrapper.find(".reasoning-toggle").trigger("click");
    expect(wrapper.find(".reasoning-toggle").attributes("aria-expanded")).toBe(
      "true",
    );
    expect(wrapper.find(".reasoning-content").exists()).toBe(true);
  });
});
