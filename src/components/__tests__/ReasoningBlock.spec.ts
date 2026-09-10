import { describe, expect, it } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ReasoningBlock from "../ReasoningBlock.vue";
import type { ThreadItem } from "../../lib/types";
import { ICON_ARROW_DOWN, ICON_ARROW_RIGHT, ICON_THINK } from "../../lib/icons";

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
  it("头部渲染思考灯泡图标", () => {
    const wrapper = mount(ReasoningBlock, {
      props: { item: reasoningItem(1, false) },
    });
    const icon = wrapper.find(".assistant-card-icon");
    expect(icon.exists()).toBe(true);
    expect(icon.find("path").attributes("d")).toBe(ICON_THINK);
  });

  it("长内容完整渲染，不再限高、无展开按钮", async () => {
    const wrapper = mount(ReasoningBlock, {
      props: { item: reasoningItem(20) },
    });
    await flushPromises();
    // 默认折叠，点击后展开
    expect(wrapper.find(".assistant-card-toggle").attributes("aria-expanded")).toBe(
      "false",
    );
    await wrapper.find(".assistant-card-toggle").trigger("click");
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
    expect(wrapper.find(".assistant-card-toggle").attributes("aria-expanded")).toBe(
      "false",
    );
    // 折叠/展开指示与会话分组同款
    expect(wrapper.find(".assistant-card-arrow path").attributes("d")).toBe(
      ICON_ARROW_RIGHT,
    );
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(wrapper.find(".assistant-card-toggle").attributes("aria-expanded")).toBe(
      "true",
    );
    expect(wrapper.find(".assistant-card-arrow path").attributes("d")).toBe(
      ICON_ARROW_DOWN,
    );
    expect(wrapper.find(".reasoning-content").exists()).toBe(true);
  });

  it("折叠态显示首行预览，点击预览展开", async () => {
    const wrapper = mount(ReasoningBlock, {
      props: { item: reasoningItem(3, false) },
    });
    await flushPromises();
    const preview = wrapper.find(".reasoning-preview");
    expect(preview.exists()).toBe(true);
    expect(preview.text()).toBe("第 1 行思考内容");
    await preview.trigger("click");
    expect(wrapper.find(".assistant-card-toggle").attributes("aria-expanded")).toBe(
      "true",
    );
    expect(wrapper.find(".reasoning-content").text()).toContain(
      "第 3 行思考内容",
    );
    expect(wrapper.find(".reasoning-preview").exists()).toBe(false);
  });
});
