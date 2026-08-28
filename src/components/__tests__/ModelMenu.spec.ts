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
import {
  activeSessionTab,
  loadModels,
  store,
  type SessionTab,
} from "../../composables/useCodex";
import { activeTabId, tabs } from "../../composables/useEditorTabs";
import { __resetSessionTabsForTest } from "../../composables/useCodex/sessionState";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";

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
    __resetSessionTabsForTest();
    tabs.push(
      makeSessionTab("s1", "t1", {
        model: "gpt-5.2-codex",
        effort: "high",
        resumedThreadId: "t1",
      }),
    );
    activeTabId.value = "s1";
    store.models = [DEFAULT_MODEL, OTHER_MODEL];
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
    const w = mount(ModelMenu);
    await w.find("button.btn.primary").trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/settings/update",
      params: { threadId: "t1", model: "gpt-5.2-codex", effort: "high" },
    });
    expect(w.emitted("close")).toBeTruthy();
  });

  it("选择默认模型后应用：model/effort 以 null 同步（恢复默认）", async () => {
    (tabs[0] as SessionTab).effort = ""; // 默认模型支持档位不强制清空；初始无强度时应用 null
    const w = mount(ModelMenu);
    await w.findAll(".option-btn")[0].trigger("click"); // 默认模型行
    await w.find("button.btn.primary").trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/settings/update",
      params: { threadId: "t1", model: null, effort: null },
    });
  });

  it("无当前会话时应用不调用 thread/settings/update", async () => {
    (tabs[0] as SessionTab).threadId = null; // 编辑态新对话：本地生效但不同步服务端
    const w = mount(ModelMenu);
    await w.findAll(".option-btn")[1].trigger("click"); // 选择其它模型
    await w.find("button.btn.primary").trigger("click");
    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(activeSessionTab()?.model).toBe("other-model");
    expect(w.emitted("close")).toBeTruthy();
  });

  it("同步失败时 toast 提示但仍关闭菜单", async () => {
    mockedInvoke.mockRejectedValue(new Error("同步失败"));
    const w = mount(ModelMenu);
    await w.find("button.btn.primary").trigger("click");
    expect(store.toast).toContain("同步失败");
    expect(w.emitted("close")).toBeTruthy();
  });

  it("历史会话未恢复：先 thread_resume 再同步 thread/settings/update", async () => {
    (tabs[0] as SessionTab).resumedThreadId = null;
    const w = mount(ModelMenu);
    await w.find("button.btn.primary").trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("thread_resume", {
      params: { threadId: "t1" },
    });
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/settings/update",
      params: { threadId: "t1", model: "gpt-5.2-codex", effort: "high" },
    });
  });

  it("thread_resume 报会话不存在：toast 且不同步、菜单关闭", async () => {
    (tabs[0] as SessionTab).resumedThreadId = null;
    mockedInvoke.mockRejectedValueOnce(new Error("thread not found: t1"));
    const w = mount(ModelMenu);
    await w.find("button.btn.primary").trigger("click");
    expect(store.toast).toContain("会话已不存在");
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "codex_rpc",
      expect.objectContaining({ method: "thread/settings/update" }),
    );
    expect(w.emitted("close")).toBeTruthy();
  });
});
