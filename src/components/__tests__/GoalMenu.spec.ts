import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { mount } from "@vue/test-utils";

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return {
    ...mod,
    setGoal: vi.fn(),
  };
});

import GoalMenu from "../GoalMenu.vue";
import { setGoal, store } from "../../composables/useCodex";

const mockedSetGoal = vi.mocked(setGoal);
let wrapper: ReturnType<typeof mount> | undefined;

describe("GoalMenu 设置目标弹层（填写即生效，无按钮）", () => {
  beforeEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
    store.goalText = null;
    store.goalStatus = null;
    mockedSetGoal.mockReset();
    mockedSetGoal.mockResolvedValue(true);
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
  });

  it("仅渲染输入框（无确定/取消按钮），打开时回填当前目标", () => {
    store.goalText = "旧目标";
    wrapper = mount(GoalMenu);
    const input = wrapper.find(".goal-input");
    expect(input.exists()).toBe(true);
    expect((input.element as HTMLTextAreaElement).value).toBe("旧目标");
    expect(wrapper.findAll("button").length).toBe(0);
    expect(wrapper.text()).toContain("Enter 或点击外部即生效");
  });

  it("Enter 提交并关闭", async () => {
    wrapper = mount(GoalMenu);
    const input = wrapper.find(".goal-input");
    await input.setValue("修复登录");
    await input.trigger("keydown", { key: "Enter" });
    expect(mockedSetGoal).toHaveBeenCalledWith("修复登录");
    const w = wrapper;
    await vi.waitFor(() => expect(w?.emitted("close")).toBeTruthy());
  });

  it("失焦（点击外部）提交并关闭", async () => {
    wrapper = mount(GoalMenu);
    const input = wrapper.find(".goal-input");
    await input.setValue("重构");
    await input.trigger("blur");
    expect(mockedSetGoal).toHaveBeenCalledWith("重构");
    const w = wrapper;
    await vi.waitFor(() => expect(w?.emitted("close")).toBeTruthy());
  });

  it("Esc 关闭且不保存，随后的失焦也不误存", async () => {
    wrapper = mount(GoalMenu);
    const input = wrapper.find(".goal-input");
    await input.setValue("不应保存");
    await input.trigger("keydown", { key: "Escape" });
    expect(wrapper.emitted("close")).toBeTruthy();
    expect(mockedSetGoal).not.toHaveBeenCalled();

    await input.trigger("blur");
    expect(mockedSetGoal).not.toHaveBeenCalled();
  });

  it("空文本 Enter 关闭但不调用 setGoal", async () => {
    wrapper = mount(GoalMenu);
    await wrapper.find(".goal-input").trigger("keydown", { key: "Enter" });
    expect(wrapper.emitted("close")).toBeTruthy();
    expect(mockedSetGoal).not.toHaveBeenCalled();
  });

  it("空文本失焦关闭但不调用 setGoal", async () => {
    wrapper = mount(GoalMenu);
    await wrapper.find(".goal-input").trigger("blur");
    expect(wrapper.emitted("close")).toBeTruthy();
    expect(mockedSetGoal).not.toHaveBeenCalled();
  });

  it("setGoal 失败时不关闭", async () => {
    mockedSetGoal.mockResolvedValue(false);
    wrapper = mount(GoalMenu);
    const input = wrapper.find(".goal-input");
    await input.setValue("目标");
    await input.trigger("keydown", { key: "Enter" });
    expect(mockedSetGoal).toHaveBeenCalledWith("目标");
    expect(wrapper.emitted("close")).toBeUndefined();
  });
});
