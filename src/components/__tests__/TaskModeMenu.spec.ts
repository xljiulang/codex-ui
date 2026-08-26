import { beforeEach, describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import TaskModeMenu from "../TaskModeMenu.vue";
import { activeSessionTab } from "../../composables/useCodex";
import type { SessionTab } from "../../composables/useCodex";
import { activeTabId, tabs } from "../../composables/useEditorTabs";
import { __resetSessionTabsForTest } from "../../composables/useCodex/sessionState";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";

describe("TaskModeMenu 任务模式菜单", () => {
  beforeEach(() => {
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
  });

  it("仅渲染执行/计划两项（无目标模式）", () => {
    const w = mount(TaskModeMenu);
    expect(w.findAll(".mode-menu-item")).toHaveLength(2);
    expect(w.text()).toContain("执行模式");
    expect(w.text()).toContain("计划模式");
    expect(w.text()).not.toContain("目标");
  });

  it("点击切换到计划模式并关闭", async () => {
    const w = mount(TaskModeMenu);
    await w.findAll(".mode-menu-item")[1].trigger("click");
    expect(activeSessionTab()?.taskMode).toBe("plan");
    expect(w.emitted("close")).toBeTruthy();
  });

  it("回合进行中点击无效（兜底）", async () => {
    (tabs[0] as SessionTab).turnActive = true;
    const w = mount(TaskModeMenu);
    await w.findAll(".mode-menu-item")[1].trigger("click");
    expect(activeSessionTab()?.taskMode).toBe("default");
    expect(w.emitted("close")).toBeUndefined();
  });
});
