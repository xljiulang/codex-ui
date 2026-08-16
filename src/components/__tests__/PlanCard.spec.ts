import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import PlanCard from "../PlanCard.vue";

describe("PlanCard Updated Plan 任务清单", () => {
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
    expect(wrapper.findAll(".plan-step")).toHaveLength(3);
    expect(wrapper.find(".plan-step.pending").text()).toContain("☐");
    expect(wrapper.find(".plan-step.in-progress").text()).toContain("◐");
    expect(wrapper.find(".plan-step.done").text()).toContain("☑");
    expect(wrapper.find(".plan-step.done .plan-step-text").text()).toBe("C");
  });

  it("无说明与空步骤时正常渲染空卡片", () => {
    const wrapper = mount(PlanCard, { props: { plan: { steps: [] } } });
    expect(wrapper.find(".plan-card").exists()).toBe(true);
    expect(wrapper.findAll(".plan-step")).toHaveLength(0);
  });
});
