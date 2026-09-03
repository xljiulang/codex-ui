import { beforeEach, describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import CollaborationModeMenu from "../CollaborationModeMenu.vue";
import { activeSessionTab } from "../../composables/useCodex";
import type { SessionTab } from "../../composables/useCodex";
import { activeTabId, tabs } from "../../composables/useEditorTabs";
import { __resetSessionTabsForTest } from "../../composables/useCodex/sessionState";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";

describe("CollaborationModeMenu 协作模式菜单", () => {
  beforeEach(() => {
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
  });

  it("仅渲染执行/计划两项（无目标模式）", () => {
    const w = mount(CollaborationModeMenu);
    expect(w.findAll(".mode-menu-item")).toHaveLength(2);
    expect(w.text()).toContain("默认模式");
    expect(w.text()).toContain("计划模式");
    expect(w.text()).not.toContain("目标");
  });

  it("点击切换到计划模式并关闭", async () => {
    const w = mount(CollaborationModeMenu);
    await w.findAll(".mode-menu-item")[1].trigger("click");
    expect(activeSessionTab()?.collaborationMode).toBe("plan");
    expect(w.emitted("close")).toBeTruthy();
  });

  it("回合进行中点击无效（兜底）", async () => {
    (tabs[0] as SessionTab).turnActive = true;
    const w = mount(CollaborationModeMenu);
    await w.findAll(".mode-menu-item")[1].trigger("click");
    expect(activeSessionTab()?.collaborationMode).toBe("default");
    expect(w.emitted("close")).toBeUndefined();
  });
});
