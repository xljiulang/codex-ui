import { describe, expect, it, beforeEach, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

import AppHeader from "../AppHeader.vue";
import { tooltipDirective } from "../../directives/tooltip";
import { activeSessionTab, store } from "../../composables/useCodex";
import {
  ICON_LAYOUT_SIDE,
  ICON_LAYOUT_SIDE_HIDDEN,
} from "../../lib/icons";
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

const h = vi.hoisted(() => {
  const win = {
    minimize: vi.fn(async () => {}),
    toggleMaximize: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    isMaximized: vi.fn(async () => false),
    onResized: vi.fn(async () => () => {}),
    startDragging: vi.fn(async () => {}),
  };
  return { shouldThrow: false, win };
});

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => {
    if (h.shouldThrow) throw new Error("not in tauri");
    return h.win;
  },
}));

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
    h.win.minimize.mockClear();
    h.win.toggleMaximize.mockClear();
    h.win.close.mockClear();
    h.win.isMaximized.mockClear();
    h.win.startDragging.mockClear();
  });

  it("点设置创建设置标签并激活", async () => {
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(tabs.some((t) => t.id === SETTINGS_TAB_ID)).toBe(true);
    expect(activeTabId.value).toBe(SETTINGS_TAB_ID);
  });

  it("设置标签已激活时再点设置：仍保持设置标签激活，不关闭", async () => {
    openSettingsTab();
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(tabs.some((t) => t.id === SETTINGS_TAB_ID)).toBe(true);
    expect(activeTabId.value).toBe(SETTINGS_TAB_ID);
    expect(activeSessionTab()?.threadId).toBe("t1");
  });

  it("设置标签存在但未激活时点设置仅激活", async () => {
    openSettingsTab();
    activateTab("s1");
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(tabs.some((t) => t.id === SETTINGS_TAB_ID)).toBe(true);
    expect(activeTabId.value).toBe(SETTINGS_TAB_ID);
  });

  it("设置按钮无 active 态", async () => {
    openSettingsTab();
    const wrapper = mountHeader();
    expect(wrapper.find('button[aria-label="设置"]').classes()).not.toContain(
      "active",
    );
  });
});

describe("AppHeader 标题栏与窗口控制", () => {
  beforeEach(() => {
    __resetTabsForTest();
    __resetSessionTabsForTest();
    h.win.minimize.mockClear();
    h.win.toggleMaximize.mockClear();
    h.win.close.mockClear();
    h.win.isMaximized.mockClear();
    h.win.startDragging.mockClear();
    store.rightPanelHidden = false;
  });

  it("头部顺序：设置 / 布局切换 / 配置快照 / 最小化 / 最大化 / 关闭", async () => {
    const wrapper = mountHeader();
    await flushPromises();
    const actions = wrapper.findAll(".header-actions > *");
    expect(actions[0].attributes("aria-label")).toBe("设置");
    expect(actions[1].attributes("aria-label")).toBe("切换布局");
    // 配置快照是纯图标按钮，位于布局切换之后、最小化之前（无文字）
    const configBtn = actions[2].find('button[aria-label="配置快照"]');
    expect(configBtn.exists()).toBe(true);
    expect(configBtn.text().trim()).toBe("");
    expect(actions[3].attributes("aria-label")).toBe("最小化");
    expect(actions[4].attributes("aria-label")).toBe("最大化");
    expect(actions[5].attributes("aria-label")).toBe("关闭");
  });

  it("点最小化调用窗口 minimize", async () => {
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="最小化"]').trigger("click");
    expect(h.win.minimize).toHaveBeenCalledTimes(1);
  });

  it("点最大化调用窗口 toggleMaximize", async () => {
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="最大化"]').trigger("click");
    expect(h.win.toggleMaximize).toHaveBeenCalledTimes(1);
  });

  it("点关闭调用窗口 close", async () => {
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="关闭"]').trigger("click");
    expect(h.win.close).toHaveBeenCalledTimes(1);
  });

  it("最大化时按钮显示还原态（aria-label=还原）", async () => {
    h.win.isMaximized.mockResolvedValue(true);
    const wrapper = mountHeader();
    await flushPromises();
    const maxBtn = wrapper.find(
      '.header-actions button[aria-label="还原"]',
    );
    expect(maxBtn.exists()).toBe(true);
  });

  it("非 Tauri 环境：标题栏仍可渲染，不抛错", async () => {
    h.shouldThrow = true;
    const wrapper = mountHeader();
    await flushPromises();
    expect(wrapper.find(".app-header").exists()).toBe(true);
    // 点击窗口按钮应静默降级，不再调用窗口 API
    await wrapper.find('button[aria-label="关闭"]').trigger("click");
    expect(h.win.close).not.toHaveBeenCalled();
  });

  it("点击窗口按钮不触发标题栏拖动", async () => {
    const wrapper = mountHeader();
    await wrapper
      .find('button[aria-label="最小化"]')
      .trigger("mousedown", { button: 0 });
    expect(h.win.startDragging).not.toHaveBeenCalled();
  });
});

describe("AppHeader 右侧面板显隐切换", () => {
  beforeEach(() => {
    __resetTabsForTest();
    __resetSessionTabsForTest();
    store.rightPanelHidden = false;
  });

  it("初始为左右布局图标（面板显示态），点击后切为隐藏态图标", async () => {
    const wrapper = mountHeader();
    const btn = wrapper.find('button[aria-label="切换布局"]');
    expect(btn.find("path").attributes("d")).toBe(ICON_LAYOUT_SIDE);
    await btn.trigger("click");
    expect(store.rightPanelHidden).toBe(true);
    expect(btn.find("path").attributes("d")).toBe(ICON_LAYOUT_SIDE_HIDDEN);
  });

  it("隐藏态再点击恢复显示（状态取反往返）", async () => {
    store.rightPanelHidden = true;
    const wrapper = mountHeader();
    const btn = wrapper.find('button[aria-label="切换布局"]');
    expect(btn.find("path").attributes("d")).toBe(ICON_LAYOUT_SIDE_HIDDEN);
    await btn.trigger("click");
    expect(store.rightPanelHidden).toBe(false);
    expect(btn.find("path").attributes("d")).toBe(ICON_LAYOUT_SIDE);
  });
});
