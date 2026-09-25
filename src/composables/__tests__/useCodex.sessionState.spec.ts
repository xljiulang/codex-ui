import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_MODEL, makeSessionTab } from "./useCodexTestHarness";
import {
  applyResumedCollaborationMode,
  applyResumedSettings,
  hydrateSessionState,
  removeSessionState,
  saveSessionState,
} from "../useCodex/sessionState";
import { effectiveSessionModelEffort } from "../useCodex/settings";
import { store } from "../useCodex/store";

describe("useCodex 会话状态持久化", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockResolvedValue(undefined);
    store.models = [];
    store.modelsLoaded = false;
    store.toast = "";
  });

  it("saveSessionState 写入线程的权限/模型/推理强度", async () => {
    const tab = makeSessionTab("s1", "t1", {
      permissionMode: "full-access",
      model: "deepseek-v4-flash",
      effort: "high",
    });
    await saveSessionState(tab);
    expect(invoke).toHaveBeenCalledWith("sessions_update", {
      threadId: "t1",
      permissionMode: "full-access",
      model: "deepseek-v4-flash",
      effort: "high",
    });
  });

  it("threadId 为空时 saveSessionState 不写入", async () => {
    await saveSessionState(makeSessionTab("s1", null));
    expect(invoke).not.toHaveBeenCalled();
  });

  it("saveSessionState 传 resolved 时用覆盖值写入", async () => {
    const tab = makeSessionTab("s1", "t1", {
      model: null,
      effort: null,
    });
    await saveSessionState(tab, { model: "deepseek-v4-flash", effort: "high" });
    expect(invoke).toHaveBeenCalledWith("sessions_update", {
      threadId: "t1",
      permissionMode: "ask-for-approval",
      model: "deepseek-v4-flash",
      effort: "high",
    });
  });

  it("hydrateSessionState 从 sessions_get 回填标签字段", async () => {
    vi.mocked(invoke).mockResolvedValue({
      permissionMode: "help-me-approve",
      model: "gpt-x",
      effort: "max",
    });
    const tab = makeSessionTab("s1", "t1");
    await hydrateSessionState(tab);
    expect(invoke).toHaveBeenCalledWith("sessions_get", { threadId: "t1" });
    expect(tab.permissionMode).toBe("help-me-approve");
    expect(tab.model).toBe("gpt-x");
    expect(tab.effort).toBe("max");
  });

  it("无记录时 hydrateSessionState 保持默认", async () => {
    vi.mocked(invoke).mockResolvedValue(null);
    const tab = makeSessionTab("s1", "t1", { permissionMode: "read-only" });
    await hydrateSessionState(tab);
    expect(tab.permissionMode).toBe("read-only");
  });

  it("模型已下架时 hydrate 回退默认并写回 + 提示", async () => {
    store.models = [DEFAULT_MODEL];
    store.modelsLoaded = true;
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === "sessions_get") {
        return {
          permissionMode: "full-access",
          model: "gone-model",
          effort: "max",
        };
      }
      return undefined;
    });
    const tab = makeSessionTab("s1", "t1");
    await hydrateSessionState(tab);
    expect(tab.model).toBe(DEFAULT_MODEL.model);
    expect(tab.effort).toBeNull();
    expect(invoke).toHaveBeenCalledWith("sessions_update", {
      threadId: "t1",
      permissionMode: "full-access",
      model: DEFAULT_MODEL.model,
      effort: null,
    });
    expect(store.toast).toContain("已回退");
  });

  it("推理强度档位下架时清空为默认并提示", async () => {
    store.models = [
      {
        ...DEFAULT_MODEL,
        supportedReasoningEfforts: [
          { reasoningEffort: "low", description: "" },
          { reasoningEffort: "high", description: "" },
        ],
      },
    ];
    store.modelsLoaded = true;
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === "sessions_get") {
        return {
          permissionMode: "ask-for-approval",
          model: DEFAULT_MODEL.model,
          effort: "max",
        };
      }
      return undefined;
    });
    const tab = makeSessionTab("s1", "t1");
    await hydrateSessionState(tab);
    expect(tab.model).toBe(DEFAULT_MODEL.model);
    expect(tab.effort).toBeNull();
    expect(store.toast).toContain("已恢复默认");
  });

  it("模型列表未加载时按原值应用且不回退", async () => {
    store.modelsLoaded = false;
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === "sessions_get") {
        return {
          permissionMode: "read-only",
          model: "later-model",
          effort: "low",
        };
      }
      return undefined;
    });
    const tab = makeSessionTab("s1", "t1");
    await hydrateSessionState(tab);
    expect(tab.model).toBe("later-model");
    expect(tab.effort).toBe("low");
    expect(store.toast).toBe("");
    expect(invoke).not.toHaveBeenCalledWith(
      "sessions_update",
      expect.anything(),
    );
  });

  it("removeSessionState 清空线程记录", async () => {
    await removeSessionState("t1");
    expect(invoke).toHaveBeenCalledWith("sessions_remove", { threadId: "t1" });
  });

  it("effectiveSessionModelEffort 解析默认模型与默认强度", () => {
    store.models = [
      {
        id: "m1",
        model: "deepseek-v4-flash",
        displayName: "DeepSeek",
        description: "",
        hidden: false,
        isDefault: true,
        supportedReasoningEfforts: [],
        defaultReasoningEffort: "high",
      },
    ];
    const tab = makeSessionTab("s1", "t1", { model: null, effort: null });
    expect(effectiveSessionModelEffort(tab)).toEqual({
      model: "deepseek-v4-flash",
      effort: "high",
    });
  });

  it("模型列表缺失时 effectiveSessionModelEffort 回退 null 不抛错", () => {
    store.models = [];
    const tab = makeSessionTab("s1", "t1", { model: null, effort: null });
    expect(effectiveSessionModelEffort(tab)).toEqual({
      model: null,
      effort: null,
    });
  });

  it("applyResumedSettings 用 thread/resume 返回值回填并落盘", () => {
    const tab = makeSessionTab("s1", "t1", { model: null, effort: null });
    applyResumedSettings(tab, { model: "gpt-5", reasoningEffort: "high" });
    expect(tab.model).toBe("gpt-5");
    expect(tab.effort).toBe("high");
    expect(invoke).toHaveBeenCalledWith("sessions_update", {
      threadId: "t1",
      permissionMode: "ask-for-approval",
      model: "gpt-5",
      effort: "high",
    });
  });

  it("applyResumedSettings 字段缺失时不覆盖、不落盘", () => {
    const tab = makeSessionTab("s1", "t1", {
      model: "gpt-5",
      effort: "high",
    });
    applyResumedSettings(tab, {});
    expect(tab.model).toBe("gpt-5");
    expect(tab.effort).toBe("high");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("applyResumedSettings：resume 模型已下架时保留本地回退值", () => {
    store.models = [{ ...DEFAULT_MODEL, defaultReasoningEffort: "high" }];
    store.modelsLoaded = true;
    const tab = makeSessionTab("s1", "t1", {
      model: DEFAULT_MODEL.model,
      effort: "high",
    });
    applyResumedSettings(tab, {
      model: "gone-model",
      reasoningEffort: "max",
    });
    expect(tab.model).toBe(DEFAULT_MODEL.model);
    expect(tab.effort).toBe("high");
    expect(store.toast).toBe("");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("applyResumedSettings：resume 模型已下架且本地为空时回退默认并提示", () => {
    store.models = [{ ...DEFAULT_MODEL, defaultReasoningEffort: "high" }];
    store.modelsLoaded = true;
    const tab = makeSessionTab("s1", "t1", { model: null, effort: null });
    applyResumedSettings(tab, {
      model: "gone-model",
      reasoningEffort: "max",
    });
    expect(tab.model).toBe(DEFAULT_MODEL.model);
    expect(tab.effort).toBe("high");
    expect(store.toast).toContain("已回退");
    expect(invoke).toHaveBeenCalledWith("sessions_update", {
      threadId: "t1",
      permissionMode: "ask-for-approval",
      model: DEFAULT_MODEL.model,
      effort: "high",
    });
  });

  it("applyResumedSettings：本地模型同样不可用时回退默认", () => {
    store.models = [DEFAULT_MODEL];
    store.modelsLoaded = true;
    const tab = makeSessionTab("s1", "t1", {
      model: "gone-local",
      effort: "max",
    });
    applyResumedSettings(tab, { model: "gone-server" });
    expect(tab.model).toBe(DEFAULT_MODEL.model);
    expect(tab.effort).toBeNull();
    expect(store.toast).toContain("已回退");
  });

  // 0.156.0 起 thread/resume 响应带 collaborationMode（服务端持久化的权威模式）
  it("applyResumedCollaborationMode：plan / default 写入标签", () => {
    const plan = makeSessionTab("s1", "t1");
    applyResumedCollaborationMode(plan, {
      collaborationMode: { mode: "plan" },
    });
    expect(plan.collaborationMode).toBe("plan");

    const back = makeSessionTab("s2", "t2", { collaborationMode: "plan" });
    applyResumedCollaborationMode(back, {
      collaborationMode: { mode: "default" },
    });
    expect(back.collaborationMode).toBe("default");
  });

  it("applyResumedCollaborationMode：字段缺失 / 非法取值 / 非对象一律不动", () => {
    // 0.149–0.155 的 resume 没有该字段
    const legacy = makeSessionTab("s1", "t1", { collaborationMode: "plan" });
    applyResumedCollaborationMode(legacy, { model: "gpt-x" });
    expect(legacy.collaborationMode).toBe("plan");

    const unknown = makeSessionTab("s2", "t2", { collaborationMode: "plan" });
    applyResumedCollaborationMode(unknown, {
      collaborationMode: { mode: "chatty" },
    });
    expect(unknown.collaborationMode).toBe("plan");

    const nullMode = makeSessionTab("s3", "t3", { collaborationMode: "plan" });
    applyResumedCollaborationMode(nullMode, { collaborationMode: null });
    expect(nullMode.collaborationMode).toBe("plan");

    const notObject = makeSessionTab("s4", "t4", { collaborationMode: "plan" });
    applyResumedCollaborationMode(notObject, "nope");
    applyResumedCollaborationMode(notObject, null);
    applyResumedCollaborationMode(notObject, undefined);
    expect(notObject.collaborationMode).toBe("plan");
  });
});
