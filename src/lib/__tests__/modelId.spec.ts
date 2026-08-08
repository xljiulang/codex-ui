import { describe, expect, it, beforeEach } from "vitest";
import { currentModelId, store } from "../../composables/useCodex";

beforeEach(() => {
  store.model = null;
  store.currentModel = "";
  store.models = [];
});

describe("currentModelId 模型取值", () => {
  it("显式选择的模型优先", () => {
    store.model = "deepseek-v4-pro";
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
