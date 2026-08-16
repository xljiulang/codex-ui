import { describe, expect, it, beforeEach } from "vitest";
import { currentModelId, store } from "../../composables/useCodex";
import { activeTabId, tabs } from "../../composables/useEditorTabs";
import { __resetSessionTabsForTest } from "../../composables/useCodex/sessionState";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";

beforeEach(() => {
  __resetSessionTabsForTest();
  store.currentModel = "";
  store.models = [];
});

describe("currentModelId 模型取值", () => {
  it("显式选择的模型优先", () => {
    tabs.push(makeSessionTab("s1", "t1", { model: "deepseek-v4-pro" }));
    activeTabId.value = "s1";
    store.currentModel = "deepseek-v4-flash";
    store.models = [
      { id: "1", model: "deepseek-v4-flash", isDefault: true },
    ] as never;
    expect(currentModelId()).toBe("deepseek-v4-pro");
  });

  it("未显式选择时回退会话已知模型", () => {
    store.currentModel = "deepseek-v4-pro";
    expect(currentModelId()).toBe("deepseek-v4-pro");
  });

  it("回退到默认模型", () => {
    store.models = [
      { id: "1", model: "deepseek-v4-flash", isDefault: true },
      { id: "2", model: "deepseek-v4-pro", isDefault: false },
    ] as never;
    expect(currentModelId()).toBe("deepseek-v4-flash");
  });

  it("模型列表为空时返回空字符串", () => {
    expect(currentModelId()).toBe("");
  });
});
