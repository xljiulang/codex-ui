import { describe, expect, it } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick, reactive } from "vue";
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

  it("标题固定为「思考过程」，不随开合变化", async () => {
    const wrapper = mount(ReasoningBlock, {
      props: { item: reasoningItem(2, false) },
    });
    await flushPromises();
    const title = wrapper.find(".assistant-card-title");
    expect(title.exists()).toBe(true);
    expect(title.text()).toBe("思考过程");
    // 展开/收起后标题文案不变，开合状态由箭头与 aria-expanded 表达
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(wrapper.find(".assistant-card-title").text()).toBe("思考过程");
    const header = wrapper.find(".assistant-card-toggle").text();
    expect(header).toContain("思考过程");
    expect(header).not.toContain("显示思考过程");
    expect(header).not.toContain("收起思考过程");
  });

  it("折叠态显示最新一行预览，点击预览展开", async () => {
    const wrapper = mount(ReasoningBlock, {
      props: { item: reasoningItem(3, false) },
    });
    await flushPromises();
    const preview = wrapper.find(".reasoning-preview");
    expect(preview.exists()).toBe(true);
    expect(preview.text()).toBe("第 3 行思考内容");
    await preview.trigger("click");
    expect(wrapper.find(".assistant-card-toggle").attributes("aria-expanded")).toBe(
      "true",
    );
    expect(wrapper.find(".reasoning-content").text()).toContain(
      "第 3 行思考内容",
    );
    expect(wrapper.find(".reasoning-preview").exists()).toBe(false);
  });

  it("折叠态预览跳过末尾的空行与空白行", async () => {
    const wrapper = mount(ReasoningBlock, {
      props: {
        item: {
          id: "r1",
          type: "reasoning",
          content: ["第 1 行内容", "第 2 行内容", "   ", ""],
          streaming: false,
        } as ThreadItem,
      },
    });
    await flushPromises();
    expect(wrapper.find(".reasoning-preview").text()).toBe("第 2 行内容");
  });

  it("折叠态预览随流式内容增长实时更新为最新一行", async () => {
    const item = reactive(reasoningItem(1, true));
    const wrapper = mount(ReasoningBlock, {
      props: { item: item as ThreadItem },
    });
    await flushPromises();
    expect(wrapper.find(".reasoning-preview").text()).toBe("第 1 行思考内容");

    (item.content as string[]).push("第 2 行思考内容", "第 3 行思考内容");
    // 流式结束：节流内容立即刷净（流式中最多每 80ms 刷新一次）
    item.streaming = false;
    await nextTick();
    expect(wrapper.find(".reasoning-preview").text()).toBe("第 3 行思考内容");
  });

  it("折叠态预览超出一行宽度时顶到行尾并标记裁剪", async () => {
    const item = reactive(reasoningItem(1, true));
    const wrapper = mount(ReasoningBlock, {
      props: { item: item as ThreadItem },
    });
    await flushPromises();
    const el = wrapper.find(".reasoning-preview").element as HTMLElement;
    // jsdom 无布局：默认 0/0 视为未溢出
    expect(el.classList.contains("is-clipped")).toBe(false);

    Object.defineProperty(el, "scrollWidth", { value: 420, configurable: true });
    Object.defineProperty(el, "clientWidth", { value: 100, configurable: true });
    Object.defineProperty(el, "scrollLeft", {
      value: 0,
      writable: true,
      configurable: true,
    });
    (item.content as string[]).push("很长的一行思考内容".repeat(20));
    item.streaming = false;
    await nextTick();
    await nextTick();
    await nextTick();

    expect(el.classList.contains("is-clipped")).toBe(true);
    expect(el.scrollLeft).toBe(420);
  });
});
