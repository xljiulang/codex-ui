import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { mount } from "@vue/test-utils";

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return {
    ...mod,
    dismissPlanPrompt: vi.fn(),
    executePlan: vi.fn(),
    exitPlanMode: vi.fn(),
  };
});

import PlanPromptBubble from "../PlanPromptBubble.vue";
import {
  dismissPlanPrompt,
  executePlan,
  exitPlanMode,
  store,
} from "../../composables/useCodex";

const mockedDismiss = vi.mocked(dismissPlanPrompt);
const mockedExecute = vi.mocked(executePlan);
const mockedExit = vi.mocked(exitPlanMode);
let wrapper: ReturnType<typeof mount> | undefined;

describe("PlanPromptBubble 计划已就绪气泡", () => {
  beforeEach(() => {
    store.planPrompt = null;
    wrapper?.unmount();
    wrapper = undefined;
    mockedDismiss.mockClear();
    mockedExecute.mockClear();
    mockedExit.mockClear();
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
  });

  it("未设置 planPrompt 时不渲染", () => {
    wrapper = mount(PlanPromptBubble);
    expect(wrapper.find(".interaction-bubble").exists()).toBe(false);
  });

  it("渲染紧凑气泡：标题、说明与三个按钮，不渲染计划正文", () => {
    store.planPrompt = {
      threadId: "t1",
      turnId: "turn-1",
      planText: "# 修复方案",
    };
    wrapper = mount(PlanPromptBubble);

    expect(wrapper.find(".interaction-bubble").exists()).toBe(true);
    expect(wrapper.find(".interaction-title").text()).toContain("计划已就绪");
    expect(wrapper.find(".plan-prompt-hint").text()).toContain(
      "完整计划见上方消息",
    );
    expect(wrapper.text()).not.toContain("# 修复方案");

    const buttons = wrapper.findAll(".interaction-foot .btn");
    expect(buttons.map((b) => b.text().trim())).toEqual([
      "待在计划",
      "退出计划模式",
      "执行计划",
    ]);
  });

  it("点击执行计划触发 executePlan", async () => {
    store.planPrompt = {
      threadId: "t1",
      turnId: "turn-1",
      planText: "# 修复方案",
    };
    wrapper = mount(PlanPromptBubble);
    const buttons = wrapper.findAll(".interaction-foot .btn");

    await buttons[2].trigger("click");
    expect(mockedExecute).toHaveBeenCalledTimes(1);
    expect(mockedDismiss).not.toHaveBeenCalled();
    expect(mockedExit).not.toHaveBeenCalled();
  });

  it("待在计划调用 dismissPlanPrompt、退出计划模式调用 exitPlanMode", async () => {
    store.planPrompt = {
      threadId: "t1",
      turnId: "turn-1",
      planText: "# 修复方案",
    };
    wrapper = mount(PlanPromptBubble);
    const buttons = wrapper.findAll(".interaction-foot .btn");

    await buttons[0].trigger("click");
    expect(mockedDismiss).toHaveBeenCalledTimes(1);

    await buttons[1].trigger("click");
    expect(mockedExit).toHaveBeenCalledTimes(1);
    expect(mockedExecute).not.toHaveBeenCalled();
  });

  it("按 Escape 等同待在计划", async () => {
    store.planPrompt = {
      threadId: "t1",
      turnId: "turn-1",
      planText: "# 修复方案",
    };
    wrapper = mount(PlanPromptBubble);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(mockedDismiss).toHaveBeenCalledTimes(1);
    expect(mockedExecute).not.toHaveBeenCalled();
  });
});
