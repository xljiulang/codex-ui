import { ensureSkills, ensureThreadPlugins, loadModels, loadSettings, refreshServer, resetToNewSession } from "../useCodex/settings";
import { store } from "../useCodex/store";
import type { ModelInfo } from "../useCodex/types";
import { __resetSessionTabsForTest, activeSessionTab } from "../useCodex/sessionState";
import { activeTabId } from "../useEditorTabs";
import { makeSessionTab, PLUGINS_RESPONSE, SKILLS_RESPONSE, resetUseCodexState, tabs } from "./useCodexTestHarness";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

const { mockWin } = vi.hoisted(() => ({
  mockWin: {
    label: "main",
    isMinimized: vi.fn().mockResolvedValue(false),
    unminimize: vi.fn(),
    setFocus: vi.fn(),
    setTitle: vi.fn(),
    setProgressBar: vi.fn(),
  },
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => mockWin),
  ProgressBarStatus: { Indeterminate: "Indeterminate", None: "None" },
}));

const mockedInvoke = vi.mocked(invoke);
const mockedListen = vi.mocked(listen);

beforeEach(() => {
  resetUseCodexState(mockedInvoke, mockedListen);
});

describe("refreshServer 服务状态同步", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    store.server = {
      connected: false,
      codexPath: null,
      logs: [],
    };
  });

  it("server_status 返回 codexPath 后写入 store.server.codexPath", async () => {
    mockedInvoke.mockResolvedValue({
      connected: true,
      codexPath: "D:/codex/codex.exe",
      logs: [],
    });
    await refreshServer();
    expect(mockedInvoke).toHaveBeenCalledWith("server_status");
    expect(store.server.codexPath).toBe("D:/codex/codex.exe");
    expect(store.server.connected).toBe(true);
  });
});
describe("ensureThreadPlugins 会话级插件缓存", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  it("为会话归一化 plugin/list：过滤未安装/禁用插件并按 id 去重", async () => {
    const tab = makeSessionTab("s1", "t1");
    mockedInvoke.mockResolvedValue(PLUGINS_RESPONSE);
    await ensureThreadPlugins(tab);
    expect(tab.plugins).toEqual({
      loaded: true,
      plugins: [
        {
          id: "documents@openai-primary-runtime",
          name: "documents",
          displayName: "Documents",
          description: "文档处理",
          path: "C:/x/documents",
          iconPath: "C:/x/documents/icon.png",
          iconUrl: "",
          brandColor: "#2563EB",
        },
        {
          id: "pdf@openai-primary-runtime",
          name: "pdf",
          displayName: "PDF",
          description: "",
          path: "C:/x/pdf",
          iconPath: "",
          iconUrl: "",
          brandColor: "",
        },
      ],
    });
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "plugin/list",
      params: {},
    });
    const skillsCalls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "skills/list",
    );
    expect(skillsCalls).toHaveLength(0);
  });

  it("同会话复用缓存不重复请求，不同会话各自缓存互不串扰", async () => {
    const tab1 = makeSessionTab("s1", "t1");
    const tab2 = makeSessionTab("s2", "t2");
    mockedInvoke.mockResolvedValue(PLUGINS_RESPONSE);
    await ensureThreadPlugins(tab1);
    await ensureThreadPlugins(tab1); // 同会话复用
    let calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "plugin/list",
    );
    expect(calls).toHaveLength(1);

    await ensureThreadPlugins(tab2); // 新会话预加载
    await ensureThreadPlugins(tab1); // 仍复用
    calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "plugin/list",
    );
    expect(calls).toHaveLength(2);
    // 两个会话缓存各自独立
    expect(tab1.plugins.plugins.length).toBeGreaterThan(0);
    expect(tab2.plugins.plugins.length).toBeGreaterThan(0);
    expect(tab1.plugins).not.toBe(tab2.plugins);
  });

  it("plugin/list 失败时该会话缓存为空且不回退 skills/list", async () => {
    const tab = makeSessionTab("s1", "t1");
    mockedInvoke.mockRejectedValue(new Error("boom"));
    await ensureThreadPlugins(tab);
    expect(tab.plugins).toEqual({
      plugins: [],
      loaded: false,
    });
    const skillsCalls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "skills/list",
    );
    expect(skillsCalls).toHaveLength(0);
  });
});
describe("ensureSkills 会话级技能缓存", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  it("归一化 skills/list：过滤禁用项并缓存到会话标签", async () => {
    const tab = makeSessionTab("s1", "t1");
    mockedInvoke.mockResolvedValue(SKILLS_RESPONSE);
    await ensureSkills(tab);
    expect(tab.skills).toEqual({
      loaded: true,
      skills: [
        {
          name: "csharp-code-rules",
          key: "csharp-code-rules",
          path: "C:/x/skills/csharp-code-rules/SKILL.md",
          desc: "C# 代码规范长描述",
          shortDesc: "C# 代码规范短描述",
        },
      ],
    });
    const calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "skills/list",
    );
    expect(calls).toHaveLength(1);
  });

  it("幂等：同一会话第二次调用不重复请求", async () => {
    const tab = makeSessionTab("s1", "t1");
    mockedInvoke.mockResolvedValue(SKILLS_RESPONSE);
    await ensureSkills(tab);
    await ensureSkills(tab);
    const calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "skills/list",
    );
    expect(calls).toHaveLength(1);
  });
});
describe("resetToNewSession 会话级缓存复位", () => {
  it("复位插件/技能缓存为未加载（下次发送重新拉取）", () => {
    const tab = makeSessionTab("s1", "t1", {
      plugins: {
        plugins: [
          {
            id: "p1",
            name: "p1",
            displayName: "P1",
            description: "",
            path: "",
            iconPath: "",
            iconUrl: "",
            brandColor: "",
          },
        ],
        loaded: true,
      },
      skills: {
        skills: [
          { name: "s1", key: "s1", path: "", desc: "", shortDesc: "" },
        ],
        loaded: true,
      },
    });
    tabs.push(tab);
    activeTabId.value = "s1";

    resetToNewSession();

    expect(tab.plugins).toEqual({ plugins: [], loaded: false });
    expect(tab.skills).toEqual({ skills: [], loaded: false });
  });
});
describe("loadSettings 默认权限初始值", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
  });

  it("启动时按持久化的默认权限设置权限模式初始值", async () => {
    mockedInvoke.mockResolvedValue({
      codex_path: null,
      sound_enabled: true,
      enter_to_send: true,
      followup_mode: "adjust",
      theme: "blue",
      default_permission: "full-access",
    });

    await loadSettings();

    expect(store.settings.default_permission).toBe("full-access");
    expect(activeSessionTab()?.permissionMode).toBe("full-access");
  });

  it("持久化值非法时回退 ask-for-approval", async () => {
    mockedInvoke.mockResolvedValue({
      codex_path: null,
      sound_enabled: true,
      enter_to_send: true,
      followup_mode: "adjust",
      theme: "blue",
      default_permission: "bogus-mode",
    });

    await loadSettings();

    expect(store.settings.default_permission).toBe("ask-for-approval");
    expect(activeSessionTab()?.permissionMode).toBe("ask-for-approval");
  });
});
describe("loadSettings 终端 Shell", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  it("启动时按持久化的 terminal_shell 加载", async () => {
    mockedInvoke.mockResolvedValue({
      codex_path: null,
      sound_enabled: true,
      enter_to_send: true,
      followup_mode: "adjust",
      theme: "blue",
      default_permission: "ask-for-approval",
      terminal_shell: "powershell",
    });

    await loadSettings();

    expect(store.settings.terminal_shell).toBe("powershell");
  });

  it("持久化值非法或缺失时回退 cmd", async () => {
    mockedInvoke.mockResolvedValue({
      codex_path: null,
      sound_enabled: true,
      enter_to_send: true,
      followup_mode: "adjust",
      theme: "blue",
      default_permission: "ask-for-approval",
      terminal_shell: "bogus",
    });

    await loadSettings();

    expect(store.settings.terminal_shell).toBe("cmd");
  });
});

describe("loadSettings 毛玻璃特效", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    delete document.documentElement.dataset.glass;
  });

  afterEach(() => {
    delete document.documentElement.dataset.glass;
  });

  it("持久化值缺失时回退默认 true 并写入 data-glass=on", async () => {
    mockedInvoke.mockResolvedValue({
      codex_path: null,
      sound_enabled: true,
      enter_to_send: true,
      followup_mode: "adjust",
      theme: "blue",
    });

    await loadSettings();

    expect(store.settings.glass_effect).toBe(true);
    expect(document.documentElement.dataset.glass).toBe("on");
  });

  it("持久化 glass_effect=false 时加载并写入 data-glass=off", async () => {
    mockedInvoke.mockResolvedValue({
      codex_path: null,
      sound_enabled: true,
      enter_to_send: true,
      followup_mode: "adjust",
      theme: "blue",
      glass_effect: false,
    });

    await loadSettings();

    expect(store.settings.glass_effect).toBe(false);
    expect(document.documentElement.dataset.glass).toBe("off");
  });
});

/** 模型列表 fixture：可指定默认标记与是否隐藏 */
function mkModel(
  slug: string,
  opts: { isDefault?: boolean; hidden?: boolean } = {},
): ModelInfo {
  return {
    id: slug,
    model: slug,
    displayName: slug.toUpperCase(),
    description: "",
    hidden: opts.hidden ?? false,
    isDefault: opts.isDefault ?? false,
    supportedReasoningEfforts: [{ reasoningEffort: "low", description: "" }],
    defaultReasoningEffort: "low",
  };
}

/** 目录没声明推理档位的条目（如 Zen 的 big-pickle）：空数组表示"未知" */
function mkModelWithoutLevels(slug: string): ModelInfo {
  return {
    ...mkModel(slug),
    supportedReasoningEfforts: [],
    defaultReasoningEffort: "",
  };
}

/** 按 codex_rpc 方法返回目录/配置响应；catalogThrows/configThrows 模拟调用失败 */
function mockModelRpc(opts: {
  catalog?: ModelInfo[] | null;
  config?: unknown;
  catalogThrows?: boolean;
  configThrows?: boolean;
}) {
  mockedInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd !== "codex_rpc") return undefined;
    const method = (args as { method?: string } | undefined)?.method;
    if (method === "model/list") {
      if (opts.catalogThrows) throw new Error("model/list failed");
      return opts.catalog === null ? {} : { data: opts.catalog ?? [] };
    }
    if (method === "config/read") {
      if (opts.configThrows) throw new Error("config/read failed");
      return opts.config ?? {};
    }
    return undefined;
  });
}

describe("loadModels 默认模型合成", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    store.models = [];
    store.modelsLoaded = false;
  });

  it("config model 命中目录：原顺序不变、命中行标默认并覆盖默认强度，config/read 带 cwd", async () => {
    mockModelRpc({
      catalog: [mkModel("a", { isDefault: true }), mkModel("b"), mkModel("c")],
      config: { config: { model: "b", model_reasoning_effort: "high" } },
    });
    await loadModels(true, "D:/repo");
    expect(store.models.map((m) => m.model)).toEqual(["a", "b", "c"]);
    expect(store.models.find((m) => m.isDefault)?.model).toBe("b");
    expect(
      store.models.find((m) => m.model === "b")?.defaultReasoningEffort,
    ).toBe("high");
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "config/read",
      params: { includeLayers: true, cwd: "D:/repo" },
    });
  });

  it("config model 未命中：置顶合成项（无档位、配置强度作默认）", async () => {
    mockModelRpc({
      catalog: [mkModel("a", { isDefault: true }), mkModel("b"), mkModel("c")],
      config: { config: { model: "d", model_reasoning_effort: "high" } },
    });
    await loadModels(true, "");
    expect(store.models.map((m) => m.model)).toEqual(["d", "a", "b", "c"]);
    expect(store.models[0]).toMatchObject({
      isDefault: true,
      displayName: "d",
      supportedReasoningEfforts: [],
      defaultReasoningEffort: "high",
    });
    expect(store.models.slice(1).every((m) => !m.isDefault)).toBe(true);
  });

  it("config model 为空：沿用服务端 isDefault 并覆盖默认强度", async () => {
    mockModelRpc({
      catalog: [mkModel("a"), mkModel("b", { isDefault: true }), mkModel("c")],
      config: { config: { model: null, model_reasoning_effort: "high" } },
    });
    await loadModels(true, "");
    expect(store.models.map((m) => m.model)).toEqual(["a", "b", "c"]);
    expect(store.models.find((m) => m.isDefault)?.model).toBe("b");
    expect(
      store.models.find((m) => m.model === "b")?.defaultReasoningEffort,
    ).toBe("high");
  });

  it("目录没声明档位的模型（空数组=未知）沿用 config 强度作默认，声明档位的不受影响", async () => {
    mockModelRpc({
      catalog: [
        mkModel("a", { isDefault: true }),
        mkModel("b"),
        mkModelWithoutLevels("big-pickle"),
      ],
      config: { config: { model: "a", model_reasoning_effort: "medium" } },
    });
    await loadModels(true, "");
    const bySlug = (slug: string) =>
      store.models.find((m) => m.model === slug)?.defaultReasoningEffort;
    expect(bySlug("a")).toBe("medium"); // 配置模型：既有覆盖规则
    expect(bySlug("big-pickle")).toBe("medium"); // 未声明档位：沿用配置强度
    expect(bySlug("b")).toBe("low"); // 声明了档位：保持条目自带默认
  });

  it("config 强度为空：未声明档位的条目默认值保持为空", async () => {
    mockModelRpc({
      catalog: [mkModel("a", { isDefault: true }), mkModelWithoutLevels("big-pickle")],
      config: { config: { model: "a" } },
    });
    await loadModels(true, "");
    expect(
      store.models.find((m) => m.model === "big-pickle")?.defaultReasoningEffort,
    ).toBe("");
  });

  it("config 读取失败：沿用服务端 isDefault，不插合成项", async () => {
    mockModelRpc({
      catalog: [mkModel("a"), mkModel("b", { isDefault: true })],
      configThrows: true,
    });
    await loadModels(true, "D:/repo");
    expect(store.models.map((m) => m.model)).toEqual(["a", "b"]);
    expect(store.models.find((m) => m.isDefault)?.model).toBe("b");
  });

  it("model/list 失败但 config model 可用：列表仅含合成项", async () => {
    mockModelRpc({
      catalogThrows: true,
      config: { config: { model: "d", model_reasoning_effort: null } },
    });
    await loadModels(true, "");
    expect(store.models.map((m) => m.model)).toEqual(["d"]);
    expect(store.models[0].isDefault).toBe(true);
  });

  it("两者都不可用：保持现有列表不变", async () => {
    store.models = [mkModel("keep", { isDefault: true })];
    mockModelRpc({ catalogThrows: true, configThrows: true });
    await loadModels(true, "");
    expect(store.models.map((m) => m.model)).toEqual(["keep"]);
  });

  it("hidden 目录项被过滤；config model 命中隐藏项时置顶", async () => {
    mockModelRpc({
      catalog: [mkModel("a", { isDefault: true }), mkModel("d", { hidden: true })],
      config: { config: { model: "d" } },
    });
    await loadModels(true, "");
    expect(store.models.map((m) => m.model)).toEqual(["d", "a"]);
    expect(store.models[0]).toMatchObject({ model: "d", isDefault: true });
  });

  it("force 重读配置：cwd 变化反映到默认项", async () => {
    mockModelRpc({
      catalog: [mkModel("a"), mkModel("b"), mkModel("c")],
      config: { config: { model: "b" } },
    });
    await loadModels(true, "D:/one");
    expect(store.models.find((m) => m.isDefault)?.model).toBe("b");
    mockModelRpc({
      catalog: [mkModel("a"), mkModel("b"), mkModel("c")],
      config: { config: { model: "c" } },
    });
    await loadModels(true, "D:/two");
    expect(store.models.find((m) => m.isDefault)?.model).toBe("c");
  });

  it("带 cwd 读取失败：重试线程无关配置并生效", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd !== "codex_rpc") return undefined;
      const method = (args as { method?: string } | undefined)?.method;
      if (method === "model/list") return { data: [mkModel("a"), mkModel("b")] };
      if (method === "config/read") {
        const cwd = (args as { params?: { cwd?: string } }).params?.cwd;
        if (cwd) throw new Error("bad cwd");
        return { config: { model: "b" } };
      }
      return undefined;
    });
    await loadModels(true, "D:/bad");
    expect(store.models.find((m) => m.isDefault)?.model).toBe("b");
    const configCalls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "config/read",
    );
    expect(configCalls).toHaveLength(2);
    expect(
      (configCalls[0][1] as { params: { cwd?: string } }).params.cwd,
    ).toBe("D:/bad");
    expect(
      (configCalls[1][1] as { params: { cwd?: string } }).params.cwd,
    ).toBeUndefined();
  });
});
