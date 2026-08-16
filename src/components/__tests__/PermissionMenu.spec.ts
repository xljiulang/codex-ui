import { beforeEach, describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import PermissionMenu from "../PermissionMenu.vue";
import { activeSessionTab } from "../../composables/useCodex";
import type { SessionTab } from "../../composables/useCodex";
import { activeTabId, tabs } from "../../composables/useEditorTabs";
import { __resetSessionTabsForTest } from "../../composables/useCodex/sessionState";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";

describe("PermissionMenu 权限模式菜单", () => {
  beforeEach(() => {
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
  });

  it("渲染三种模式并高亮当前模式", () => {
    (tabs[0] as SessionTab).permissionMode = "full-access";
    const w = mount(PermissionMenu);
    expect(w.findAll(".mode-menu-item")).toHaveLength(3);
    expect(w.text()).toContain("请求批准");
    expect(w.text()).toContain("帮我批准");
    expect(w.text()).toContain("完全访问权限");
    expect(w.find(".mode-menu-item.selected").text()).toContain("完全访问权限");
  });

  it("点击切换权限模式并关闭", async () => {
    const w = mount(PermissionMenu);
    await w.findAll(".mode-menu-item")[2].trigger("click");
    expect(activeSessionTab()?.permissionMode).toBe("full-access");
    expect(w.emitted("close")).toBeTruthy();
  });

  it("切换权限模式并关闭", async () => {
    const w = mount(PermissionMenu);
    await w.findAll(".mode-menu-item")[1].trigger("click");
    expect(activeSessionTab()?.permissionMode).toBe("help-me-approve");
    expect(w.emitted("close")).toBeTruthy();
  });
});
