import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import PlanTextCard from "../PlanTextCard.vue";
import { ICON_ARROW_DOWN, ICON_ARROW_RIGHT, ICON_PLAN } from "../../lib/icons";

afterEach(() => {
  vi.restoreAllMocks();
});

describe("PlanTextCard 计划文本卡片（标题取自计划本身）", () => {
  it("头部渲染计划内容图标", () => {
    const wrapper = mount(PlanTextCard, {
      props: { planText: "# 方案\n- 步骤1" },
    });
    const icon = wrapper.find(".assistant-card-icon");
    expect(icon.exists()).toBe(true);
    expect(icon.find("path").attributes("d")).toBe(ICON_PLAN);
  });

  it("默认折叠：标题来自计划首个标题行，正文不可见", () => {
    const wrapper = mount(PlanTextCard, {
      props: { planText: "# 方案\n- 步骤1" },
    });
    expect(wrapper.text()).toContain("方案");
    expect(wrapper.find(".assistant-card-body").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("步骤1");
  });

  it("无标题行时回退标题 计划，正文可展开", async () => {
    const wrapper = mount(PlanTextCard, {
      props: { planText: "直接是内容" },
    });
    expect(wrapper.find(".assistant-card-title").text()).toBe("计划");
    expect(wrapper.find(".assistant-card-body").exists()).toBe(false);
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(wrapper.text()).toContain("直接是内容");
  });

  it("defaultOpen 为 true 时初始展开", () => {
    const wrapper = mount(PlanTextCard, {
      props: { planText: "# 方案\n- 步骤1", defaultOpen: true },
    });
    expect(wrapper.find(".assistant-card-body").exists()).toBe(true);
  });

  it("点击展开显示正文，再点收起", async () => {
    const wrapper = mount(PlanTextCard, {
      props: { planText: "# 方案\n- 步骤1" },
    });
    // 折叠态为右箭头（与会话分组一致）
    expect(wrapper.find(".assistant-card-arrow path").attributes("d")).toBe(
      ICON_ARROW_RIGHT,
    );
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(wrapper.find(".assistant-card-arrow path").attributes("d")).toBe(
      ICON_ARROW_DOWN,
    );
    expect(wrapper.find(".assistant-card-body").exists()).toBe(true);
    expect(wrapper.text()).toContain("步骤1");
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(wrapper.find(".assistant-card-arrow path").attributes("d")).toBe(
      ICON_ARROW_RIGHT,
    );
    expect(wrapper.find(".assistant-card-body").exists()).toBe(false);
  });

  it("复制按钮把完整原始 Markdown（含标题）传给剪贴板并短暂显示已复制", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    const wrapper = mount(PlanTextCard, {
      props: { planText: "# 标题\n- 步骤" },
    });
    await wrapper.find(".copy-btn").trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenCalledWith("# 标题\n- 步骤");
    expect(wrapper.text()).toContain("已复制");
  });

  it("空 planText 容错：无复制按钮，仍可折叠", async () => {
    const wrapper = mount(PlanTextCard, { props: { planText: "" } });
    expect(wrapper.find(".assistant-card-toggle").exists()).toBe(true);
    expect(wrapper.find(".copy-btn").exists()).toBe(false);
  });
});
