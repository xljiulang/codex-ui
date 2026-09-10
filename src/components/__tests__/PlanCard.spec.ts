import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import PlanCard from "../PlanCard.vue";
import {
  ICON_ARROW_DOWN,
  ICON_ARROW_RIGHT,
  ICON_CHECKLIST,
} from "../../lib/icons";

describe("PlanCard Updated Plan 任务清单", () => {
  it("头部渲染计划单勾选清单图标", () => {
    const wrapper = mount(PlanCard, {
      props: {
        plan: { steps: [{ step: "A", status: "pending" }] },
      },
    });
    const icon = wrapper.find(".assistant-card-icon");
    expect(icon.exists()).toBe(true);
    expect(icon.find("path").attributes("d")).toBe(ICON_CHECKLIST);
  });

  it("渲染说明与三种状态的步骤", () => {
    const wrapper = mount(PlanCard, {
      props: {
        plan: {
          explanation: "执行计划",
          steps: [
            { step: "A", status: "pending" },
            { step: "B", status: "inProgress" },
            { step: "C", status: "completed" },
          ],
        },
      },
    });
    expect(wrapper.text()).toContain("执行计划");
    expect(wrapper.find(".assistant-card-title").text()).toBe("计划");
    expect(wrapper.find(".assistant-card-sub").text()).toBe("3 步 · 1 完成");
    expect(
      wrapper.find(".assistant-card-toggle").attributes("aria-expanded"),
    ).toBe("true");
    expect(wrapper.findAll(".plan-step")).toHaveLength(3);
    expect(wrapper.find(".plan-step.pending").text()).toContain("☐");
    expect(wrapper.find(".plan-step.in-progress").text()).toContain("◐");
    expect(wrapper.find(".plan-step.done").text()).toContain("☑");
    expect(wrapper.find(".plan-step.done .plan-step-text").text()).toBe("C");
  });

  it("无说明与空步骤时正常渲染空卡片", () => {
    const wrapper = mount(PlanCard, { props: { plan: { steps: [] } } });
    expect(wrapper.find(".assistant-card").exists()).toBe(true);
    expect(wrapper.find(".assistant-card-sub").exists()).toBe(false);
    expect(wrapper.findAll(".plan-step")).toHaveLength(0);
  });

  it("点击头部折叠/展开并同步 aria-expanded", async () => {
    const wrapper = mount(PlanCard, {
      props: {
        plan: {
          steps: [
            { step: "A", status: "pending" },
            { step: "B", status: "completed" },
          ],
        },
      },
    });
    expect(wrapper.findAll(".plan-step")).toHaveLength(2);
    // 展开指示与会话分组同款：展开为下箭头
    expect(wrapper.find(".assistant-card-arrow path").attributes("d")).toBe(
      ICON_ARROW_DOWN,
    );
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(
      wrapper.find(".assistant-card-toggle").attributes("aria-expanded"),
    ).toBe("false");
    expect(wrapper.find(".assistant-card-arrow path").attributes("d")).toBe(
      ICON_ARROW_RIGHT,
    );
    expect(wrapper.findAll(".plan-step")).toHaveLength(0);
    await wrapper.find(".assistant-card-toggle").trigger("click");
    expect(
      wrapper.find(".assistant-card-toggle").attributes("aria-expanded"),
    ).toBe("true");
    expect(wrapper.find(".assistant-card-arrow path").attributes("d")).toBe(
      ICON_ARROW_DOWN,
    );
    expect(wrapper.findAll(".plan-step")).toHaveLength(2);
  });
});
