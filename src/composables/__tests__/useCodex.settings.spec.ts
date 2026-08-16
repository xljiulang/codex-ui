import { ensureSkills, ensureThreadPlugins, loadSettings, refreshServer } from "../useCodex/settings";
import { store } from "../useCodex/store";
import { __resetSessionTabsForTest, activeSessionTab } from "../useCodex/sessionState";
import { activeTabId } from "../useEditorTabs";
import { makeSessionTab, PLUGINS_RESPONSE, SKILLS_RESPONSE, resetUseCodexState, tabs } from "./useCodexTestHarness";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, it, vi } from "vitest";

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
      startupWorkspace: "",
      codexPath: null,
      logs: [],
    };
  });

  it("server_status 返回 codexPath 后写入 store.server.codexPath", async () => {
    mockedInvoke.mockResolvedValue({
      connected: true,
      startupWorkspace: "D:/repo",
      codexPath: "D:/codex/codex.exe",
      logs: [],
    });
    await refreshServer();
    expect(mockedInvoke).toHaveBeenCalledWith("server_status");
    expect(store.server.codexPath).toBe("D:/codex/codex.exe");
    expect(store.server.connected).toBe(true);
    expect(store.server.startupWorkspace).toBe("D:/repo");
  });
});
describe("ensureThreadPlugins 对话级插件缓存", () => {
  beforeEach(() => {
    store.threadPlugins = {};
    mockedInvoke.mockReset();
  });

  it("为对话归一化 plugin/list：过滤未安装/禁用插件并按 id 去重", async () => {
    mockedInvoke.mockResolvedValue(PLUGINS_RESPONSE);
    await ensureThreadPlugins("t1");
    expect(store.threadPlugins["t1"]).toEqual({
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

  it("同对话复用缓存不重复请求，不同对话各自缓存", async () => {
    mockedInvoke.mockResolvedValue(PLUGINS_RESPONSE);
    await ensureThreadPlugins("t1");
    await ensureThreadPlugins("t1"); // 同对话复用
    let calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "plugin/list",
    );
    expect(calls).toHaveLength(1);

    await ensureThreadPlugins("t2"); // 新对话预加载
    await ensureThreadPlugins("t1"); // 仍复用
    calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "plugin/list",
    );
    expect(calls).toHaveLength(2);
  });

  it("plugin/list 失败时该对话缓存为空且不回退 skills/list", async () => {
    mockedInvoke.mockRejectedValue(new Error("boom"));
    await ensureThreadPlugins("t1");
    expect(store.threadPlugins["t1"]).toEqual({
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
describe("ensureSkills 技能全局缓存", () => {
  beforeEach(() => {
    store.skills = [];
    store.skillsLoaded = false;
    mockedInvoke.mockReset();
  });

  it("归一化 skills/list：过滤禁用项并缓存", async () => {
    mockedInvoke.mockResolvedValue(SKILLS_RESPONSE);
    await ensureSkills();
    expect(store.skillsLoaded).toBe(true);
    expect(store.skills).toEqual([
      {
        name: "csharp-code-rules",
        key: "csharp-code-rules",
        path: "C:/x/skills/csharp-code-rules/SKILL.md",
        desc: "C# 代码规范长描述",
        shortDesc: "C# 代码规范短描述",
      },
    ]);
    const calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "skills/list",
    );
    expect(calls).toHaveLength(1);
  });

  it("幂等：第二次调用不重复请求", async () => {
    mockedInvoke.mockResolvedValue(SKILLS_RESPONSE);
    await ensureSkills();
    await ensureSkills();
    const calls = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "skills/list",
    );
    expect(calls).toHaveLength(1);
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
describe("loadSettings 记忆模式", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  it("启动时按持久化的记忆模式加载", async () => {
    mockedInvoke.mockResolvedValue({
      codex_path: null,
      sound_enabled: true,
      enter_to_send: true,
      followup_mode: "adjust",
      theme: "blue",
      default_permission: "ask-for-approval",
      memory_mode: "enabled",
    });

    await loadSettings();

    expect(store.settings.memory_mode).toBe("enabled");
  });

  it("持久化值非法或缺失时回退 disabled", async () => {
    mockedInvoke.mockResolvedValue({
      codex_path: null,
      sound_enabled: true,
      enter_to_send: true,
      followup_mode: "adjust",
      theme: "blue",
      default_permission: "ask-for-approval",
      memory_mode: "bogus",
    });

    await loadSettings();

    expect(store.settings.memory_mode).toBe("disabled");
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
      memory_mode: "disabled",
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
      memory_mode: "disabled",
      terminal_shell: "bogus",
    });

    await loadSettings();

    expect(store.settings.terminal_shell).toBe("cmd");
  });
});
