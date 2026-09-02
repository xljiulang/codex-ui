import { beforeEach, describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import PermissionMenu from "../PermissionMenu.vue";
import { activeSessionTab } from "../../composables/useCodex";
import type { SessionTab } from "../../composables/useCodex";
import { activeTabId, tabs } from "../../composables/useEditorTabs";
import { __resetSessionTabsForTest } from "../../composables/useCodex/sessionState";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";

/** 按菜单项文本定位权限模式菜单项（避免因排序变化而依赖索引） */
function modeItem(wrapper: ReturnType<typeof mount>, label: string) {
  return wrapper
    .findAll(".mode-menu-item")
    .find((w) => w.text().includes(label))!;
}

describe("PermissionMenu 权限模式菜单", () => {
  beforeEach(() => {
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
  });

  it("渲染四种模式并高亮当前模式", () => {
    (tabs[0] as SessionTab).permissionMode = "full-access";
    const w = mount(PermissionMenu);
    expect(w.findAll(".mode-menu-item")).toHaveLength(4);
    expect(w.text()).toContain("只读访问");
    expect(w.text()).toContain("请求批准");
    expect(w.text()).toContain("帮我批准");
    expect(w.text()).toContain("完全访问");
    expect(w.text()).toContain("不受限制地访问互联网和您电脑的任何文件");
    expect(w.find(".mode-menu-item.selected").text()).toContain("完全访问");
  });

  it("点击只读访问切换权限模式并关闭", async () => {
    const w = mount(PermissionMenu);
    await modeItem(w, "只读访问").trigger("click");
    expect(activeSessionTab()?.permissionMode).toBe("read-only");
    expect(w.emitted("close")).toBeTruthy();
  });

  it("点击完全访问切换权限模式并关闭", async () => {
    const w = mount(PermissionMenu);
    await modeItem(w, "完全访问").trigger("click");
    expect(activeSessionTab()?.permissionMode).toBe("full-access");
    expect(w.emitted("close")).toBeTruthy();
  });

  it("点击帮我批准切换权限模式并关闭", async () => {
    const w = mount(PermissionMenu);
    await modeItem(w, "帮我批准").trigger("click");
    expect(activeSessionTab()?.permissionMode).toBe("help-me-approve");
    expect(w.emitted("close")).toBeTruthy();
  });

  it("回合进行中点击模式项不改变权限模式", async () => {
    (tabs[0] as SessionTab).turnActive = true;
    const w = mount(PermissionMenu);
    await modeItem(w, "完全访问").trigger("click");
    expect(activeSessionTab()?.permissionMode).not.toBe("full-access");
    expect(w.emitted("close")).toBeFalsy();
  });
});
