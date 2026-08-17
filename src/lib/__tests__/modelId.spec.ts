import { describe, expect, it, beforeEach } from "vitest";
import { currentModelId, store } from "../../composables/useCodex";
import { activeTabId, tabs } from "../../composables/useEditorTabs";
import { __resetSessionTabsForTest } from "../../composables/useCodex/sessionState";
import type { SessionTab } from "../../composables/useCodex/types";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";

beforeEach(() => {
  __resetSessionTabsForTest();
  store.models = [];
});

describe("currentModelId 模型取值", () => {
  it("显式选择的模型优先", () => {
    tabs.push(makeSessionTab("s1", "t1", { model: "deepseek-v4-pro" }));
    activeTabId.value = "s1";
    store.models = [
      { id: "1", model: "deepseek-v4-flash", isDefault: true },
    ] as never;
    expect(currentModelId()).toBe("deepseek-v4-pro");
  });

  it("标签 model 为 null 时回退默认模型", () => {
    tabs.push(makeSessionTab("s1", "t1", { model: null }));
    activeTabId.value = "s1";
    store.models = [
      { id: "1", model: "deepseek-v4-flash", isDefault: true },
    ] as never;
    expect(currentModelId()).toBe("deepseek-v4-flash");
  });

  it("后台标签 model 为 null 时不读活动标签模型，直接回退默认模型", () => {
    tabs.push(makeSessionTab("s1", "t1", { model: "gpt-5" }));
    tabs.push(makeSessionTab("s2", "t2", { model: null }));
    activeTabId.value = "s1";
    store.models = [
      { id: "1", model: "deepseek-v4-flash", isDefault: true },
    ] as never;
    expect(currentModelId(tabs[1] as SessionTab)).toBe("deepseek-v4-flash");
  });

  it("没有默认模型时回退列表第一条", () => {
    store.models = [
      { id: "1", model: "deepseek-v4-flash", isDefault: false },
      { id: "2", model: "deepseek-v4-pro", isDefault: false },
    ] as never;
    expect(currentModelId()).toBe("deepseek-v4-flash");
  });

  it("模型列表为空且标签未选模型时抛异常", () => {
    expect(() => currentModelId()).toThrow("当前没有可用模型");
  });
});
