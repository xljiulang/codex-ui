import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, loadModels: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import ModelMenu from "../ModelMenu.vue";
import { loadModels, store } from "../../composables/useCodex";

const mockedInvoke = vi.mocked(invoke);
const mockedLoadModels = vi.mocked(loadModels);

const DEFAULT_MODEL = {
  id: "m1",
  model: "gpt-5.2-codex",
  displayName: "Codex",
  description: "默认模型",
  hidden: false,
  isDefault: true,
  supportedReasoningEfforts: [
    { reasoningEffort: "low", description: "低" },
    { reasoningEffort: "high", description: "高" },
  ],
  defaultReasoningEffort: "low",
} as (typeof store.models)[number];

const OTHER_MODEL = {
  id: "m2",
  model: "other-model",
  displayName: "Other",
  description: "",
  hidden: false,
  isDefault: false,
  supportedReasoningEfforts: [],
  defaultReasoningEffort: "",
} as (typeof store.models)[number];

describe("ModelMenu 模型与推理强度", () => {
  beforeEach(() => {
    store.models = [DEFAULT_MODEL, OTHER_MODEL];
    store.model = null;
    store.effort = null;
    store.currentThreadId = null;
    store.toast = "";
    mockedInvoke.mockReset();
    mockedLoadModels.mockReset();
  });

  afterEach(() => {
    store.models = [];
  });

  it("挂载时加载模型列表，渲染模型与默认标签", async () => {
    const w = mount(ModelMenu);
    await flushPromises();
    expect(mockedLoadModels).toHaveBeenCalled();
    expect(w.text()).toContain("Codex");
    expect(w.text()).toContain("Other");
    expect(w.text()).toContain("默认");
  });

  it("推理强度跟随所选模型（默认模型带 low/high 档位）", async () => {
    const w = mount(ModelMenu);
    expect(w.text()).toContain("low");
    expect(w.text()).toContain("high");
  });

  it("应用模型/强度：有当前会话时立即同步 thread/settings/update", async () => {
    store.currentThreadId = "t1";
    store.model = "gpt-5.2-codex";
    store.effort = "high";
    const w = mount(ModelMenu);
    await w.find("button.btn.primary").trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/settings/update",
      params: { threadId: "t1", model: "gpt-5.2-codex", effort: "high" },
    });
    expect(w.emitted("close")).toBeTruthy();
  });

  it("选择默认模型后应用：model/effort 以 null 同步（恢复默认）", async () => {
    store.currentThreadId = "t1";
    store.model = "other-model";
    const w = mount(ModelMenu);
    await w.findAll(".option-btn")[0].trigger("click"); // 默认模型行
    await w.find("button.btn.primary").trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/settings/update",
      params: { threadId: "t1", model: null, effort: null },
    });
  });

  it("无当前会话时应用不调用 thread/settings/update", async () => {
    store.model = "other-model";
    const w = mount(ModelMenu);
    await w.find("button.btn.primary").trigger("click");
    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(store.model).toBe("other-model");
    expect(w.emitted("close")).toBeTruthy();
  });

  it("同步失败时 toast 提示但仍关闭菜单", async () => {
    store.currentThreadId = "t1";
    store.model = "other-model";
    mockedInvoke.mockRejectedValue(new Error("同步失败"));
    const w = mount(ModelMenu);
    await w.find("button.btn.primary").trigger("click");
    expect(store.toast).toContain("同步失败");
    expect(w.emitted("close")).toBeTruthy();
  });
});
