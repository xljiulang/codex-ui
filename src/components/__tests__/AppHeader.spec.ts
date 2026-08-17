import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, pickAndOpenNewSession: vi.fn() };
});

import AppHeader from "../AppHeader.vue";
import { tooltipDirective } from "../../directives/tooltip";
import {
  activeSessionTab,
  pickAndOpenNewSession,
} from "../../composables/useCodex";
import {
  activateTab,
  activeTabId,
  openSettingsTab,
  SETTINGS_TAB_ID,
  tabs as _tabs,
} from "../../composables/useEditorTabs";
import { __resetTabsForTest } from "../../composables/useTabs";
import { __resetSessionTabsForTest } from "../../composables/useCodex/sessionState";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";
import type { SessionTab } from "../../composables/useCodex";

const tabs = _tabs as unknown as SessionTab[];

const mockedPickAndOpenNewSession = vi.mocked(pickAndOpenNewSession);

function mountHeader() {
  return mount(AppHeader, {
    global: { directives: { tooltip: tooltipDirective } },
  });
}

describe("AppHeader 导航", () => {
  beforeEach(() => {
    __resetTabsForTest();
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    mockedPickAndOpenNewSession.mockClear();
  });

  it("点设置创建设置标签并激活", async () => {
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(tabs.some((t) => t.id === SETTINGS_TAB_ID)).toBe(true);
    expect(activeTabId.value).toBe(SETTINGS_TAB_ID);
  });

  it("设置标签已激活时再点设置关闭，回到原对话且不新建/不中断", async () => {
    openSettingsTab();
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(tabs.some((t) => t.id === SETTINGS_TAB_ID)).toBe(false);
    expect(activeSessionTab()?.threadId).toBe("t1");
    expect(mockedPickAndOpenNewSession).not.toHaveBeenCalled();
  });

  it("设置标签存在但未激活时点设置仅激活", async () => {
    openSettingsTab();
    activateTab("s1");
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(tabs.some((t) => t.id === SETTINGS_TAB_ID)).toBe(true);
    expect(activeTabId.value).toBe(SETTINGS_TAB_ID);
  });

  it("点新建会话：调用共享新建入口（含目录选择）", async () => {
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="新建会话"]').trigger("click");
    expect(mockedPickAndOpenNewSession).toHaveBeenCalledTimes(1);
  });

  it("设置按钮无 active 态", async () => {
    openSettingsTab();
    const wrapper = mountHeader();
    expect(wrapper.find('button[aria-label="设置"]').classes()).not.toContain(
      "active",
    );
  });
});

describe("AppHeader 图标布局", () => {
  it("工作目录按钮已移除，头部只剩新建会话与设置两个图标按钮", () => {
    const wrapper = mountHeader();
    expect(wrapper.find("button.brand-cwd").exists()).toBe(false);
    expect(wrapper.find(".cwd-group").exists()).toBe(false);
    const actions = wrapper.findAll(".header-actions > *");
    expect(actions.map((a) => a.attributes("aria-label"))).toEqual([
      "新建会话",
      "设置",
    ]);
  });
});
