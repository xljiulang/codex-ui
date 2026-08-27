import { deleteThread, newEmptyChat, openHistorySession, openNewSession, pickAndOpenNewSession, sendPrompt } from "../useCodex/actions";
import { __resetSessionTabsForTest, activeSessionTab } from "../useCodex/sessionState";
import { store } from "../useCodex/store";
import {
  activeTabId,
  openSettingsTab,
  SETTINGS_TAB_ID,
} from "../useEditorTabs";
import { DEFAULT_MODEL, makeSessionTab, PLUGINS_RESPONSE, resetUseCodexState, SKILLS_RESPONSE, tabs } from "./useCodexTestHarness";
import { flushPromises } from "@vue/test-utils";
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
  store.models = [DEFAULT_MODEL];
});

describe("主窗口标题跟随活动 tab 标题", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockWin.setTitle.mockClear();
    __resetSessionTabsForTest();
    store.server = {
      connected: false,
      startupWorkspace: "D:/repo",
      codexPath: null,
      logs: [],
    };
    store.threads = [];
  });


  it("新建会话后窗口标题跟随会话标签标题", async () => {
    await newEmptyChat("D:/projects/B");
    expect(activeSessionTab()?.newChatWorkspace).toBe("D:/projects/B");
    await flushPromises();
    expect(mockWin.setTitle).toHaveBeenCalledWith("B / 新建会话");
  });
});
describe("主窗口标题跟随活动 tab 标题", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockWin.setTitle.mockClear();
    __resetSessionTabsForTest();
    store.server = {
      connected: false,
      startupWorkspace: "D:/repo",
      codexPath: null,
      logs: [],
    };
    store.threads = [];
  });


  it("切换会话标签时窗口标题跟随新 tab 标题", async () => {
    await newEmptyChat();
    const first = activeTabId.value;
    await newEmptyChat("D:/projects/B");
    expect(activeTabId.value).not.toBe(first);
    await flushPromises();
    expect(mockWin.setTitle).toHaveBeenCalledWith("B / 新建会话");
  });
});
describe("队列模式下发送提示", () => {
  beforeEach(() => {
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1", { turnActive: true }));
    activeTabId.value = "s1";
    store.settings.followup_mode = "queue";
    store.toast = "";
    activeSessionTab()?.attachments.splice(0);
  });

  it("回合进行中且为队列模式：消息入队并提示", async () => {
    const before = store.userSendRev;
    await sendPrompt("第二条消息");
    // 手动发送标记递增（ChatView 据此强制吸底）；队列落地时不走 sendPrompt，不递增
    expect(store.userSendRev).toBe(before + 1);
    expect(tabs[0].followupQueue).toHaveLength(1);
    expect(tabs[0].followupQueue[0].text).toBe("第二条消息");
    expect(store.toast).toContain("已加入队列");
  });
});
describe("会话标签状态与事件路由", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.workspace = null;
    store.interactions = [];
    store.threads = [];
  });


  it("新建会话发送首条消息：标题先取消息内容并标记首条消息名", async () => {
    tabs.push(
      makeSessionTab("s1", null, { newChatWorkspace: "D:/repo" }),
    );
    activeTabId.value = "s1";
    store.server.startupWorkspace = "D:/repo";
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "thread_start") {
        return Promise.resolve({
          thread: { id: "t1", name: null },
          model: "gpt-5",
        });
      }
      if (cmd === "thread_set_name") return Promise.resolve({});
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "nt1" } });
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      if (cmd === "codex_rpc") {
        const method = (args as { params?: { method?: string } })?.params
          ?.method;
        if (method === "thread/memoryMode/set") return Promise.resolve({});
      }
      return Promise.resolve(undefined);
    });

    await sendPrompt("帮我修复登录页面报错");
    expect(activeSessionTab()?.threadId).toBe("t1");
    expect(activeSessionTab()?.name).toBe("帮我修复登录页面报错");
    expect(tabs[0].name).toBe("帮我修复登录页面报错");
    expect(tabs[0].nameIsFirstMessage).toBe(true);
    // 模型只存在于标签且由用户显式选择写入：thread_start 返回的 res.model 不回填
    expect(activeSessionTab()?.model).toBeNull();
    expect(mockedInvoke).toHaveBeenCalledWith("thread_set_name", {
      threadId: "t1",
      name: "帮我修复登录页面报错",
    });
  });

  it("新建会话短首条消息：thread_list 未返回时也即时同步历史面板", async () => {
    tabs.push(
      makeSessionTab("s1", null, { newChatWorkspace: "D:/repo" }),
    );
    activeTabId.value = "s1";
    store.server.startupWorkspace = "D:/repo";
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "thread_start") {
        return Promise.resolve({
          thread: { id: "t1", name: null },
          model: "gpt-5",
        });
      }
      if (cmd === "thread_set_name") return Promise.resolve({});
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "nt1" } });
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      if (cmd === "codex_rpc") {
        const method = (args as { params?: { method?: string } })?.params
          ?.method;
        if (method === "thread/memoryMode/set") return Promise.resolve({});
      }
      return Promise.resolve(undefined);
    });

    await sendPrompt("ABCDE");
    expect(activeSessionTab()?.threadId).toBe("t1");
    expect(activeSessionTab()?.name).toBe("ABCDE");
    // thread_list 返回空（新线程尚未可列）时，本地兜底仍同步历史面板
    const found = store.threads.find((t) => t.id === "t1");
    expect(found).toBeTruthy();
    expect(found?.name).toBe("ABCDE");
    expect(store.threads[0]?.id).toBe("t1");
  });
});
describe("会话标签状态与事件路由", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.workspace = null;
    store.interactions = [];
    store.threads = [];
  });


  it("deleteThread：删除唯一标签后允许 0 标签并复位 live 字段", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    mockedInvoke.mockResolvedValue(undefined);
    await deleteThread("t1");
    expect(tabs).toHaveLength(0);
    expect(activeTabId.value).toBe("");
    expect(activeSessionTab()).toBeNull();
  });

  it("deleteThread：已绑定微信的会话先解除绑定再删除", async () => {
    store.wechat = {
      running: true,
      connection: "connected",
      detail: null,
      qrContent: null,
      pendingThreadId: null,
      queued: 0,
      busy: false,
      bindings: [
        { threadId: "t1", accountId: "bot-1", connection: "connected" },
      ],
    };
    mockedInvoke.mockResolvedValue(undefined);
    await deleteThread("t1");
    const calls = mockedInvoke.mock.calls.map(([c]) => c as string);
    const unbindIdx = calls.indexOf("wechat_unbind");
    const deleteIdx = calls.indexOf("thread_delete");
    expect(unbindIdx).toBeGreaterThanOrEqual(0);
    expect(deleteIdx).toBeGreaterThan(unbindIdx);
    store.wechat = null;
  });
});
describe("会话标签状态与事件路由", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.workspace = null;
    store.interactions = [];
    store.threads = [];
  });


  it("pickAndOpenNewSession：弹目录选择（初始为 workspace），选中后新建会话", async () => {
    store.server.startupWorkspace = "D:/repo";
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "pick_directory") {
        return Promise.resolve("D:/project");
      }
      return Promise.resolve(undefined);
    });
    await pickAndOpenNewSession();
    expect(mockedInvoke).toHaveBeenCalledWith("pick_directory", {
      initialDir: "D:/repo",
    });
    expect(tabs).toHaveLength(1);
    expect(tabs[0].threadId).toBeNull();
    expect(tabs[0].newChatWorkspace).toBe("D:/project");
  });
});
describe("会话标签状态与事件路由", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.workspace = null;
    store.interactions = [];
    store.threads = [];
  });


  it("pickAndOpenNewSession：有会话时初始目录为线程 cwd", async () => {
    store.server.startupWorkspace = "D:/repo";
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1", { workspace: "D:/session" }));
    activeTabId.value = "s1";
    mockedInvoke.mockResolvedValue("D:/project");
    await pickAndOpenNewSession();
    expect(mockedInvoke).toHaveBeenCalledWith("pick_directory", {
      initialDir: "D:/session",
    });
  });
});
describe("会话标签状态与事件路由", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.workspace = null;
    store.interactions = [];
    store.threads = [];
  });


  it("pickAndOpenNewSession：取消选择不新建", async () => {
    store.server.startupWorkspace = "D:/repo";
    mockedInvoke.mockResolvedValue(null);
    await pickAndOpenNewSession();
    expect(tabs).toHaveLength(0);
  });
});
describe("会话标签状态与事件路由", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.workspace = null;
    store.interactions = [];
    store.threads = [];
  });


  it("pickAndOpenNewSession：选择器打开期间防重入", async () => {
    let resolveDir!: (v: string | null) => void;
    mockedInvoke.mockImplementation(
      () =>
        new Promise<string | null>((resolve) => {
          resolveDir = resolve;
        }),
    );
    const p1 = pickAndOpenNewSession();
    const p2 = pickAndOpenNewSession();
    resolveDir(null);
    await Promise.all([p1, p2]);
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
  });
});
describe("openNewSession / openHistorySession 统一收尾", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.confirm = null;
    store.panelTab = "history";
  });

  it("openNewSession 成功后：设置标签不再激活、聚焦输入框、切回资源 Tab", async () => {
    openSettingsTab();
    const focus = vi.fn();
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ = {
      commands: { focus },
    };
    try {
      await openNewSession("D:/projects/B");
      expect(activeSessionTab()?.newChatWorkspace).toBe("D:/projects/B");
      expect(activeTabId.value).not.toBe(SETTINGS_TAB_ID);
      expect(store.panelTab).toBe("history");
      expect(focus).toHaveBeenCalledTimes(1);
    } finally {
      delete (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__;
    }
  });

  it("openNewSession：进行中会话不弹确认（会话多开），直接新建标签并收尾", async () => {
    openSettingsTab();
    const focus = vi.fn();
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ = {
      commands: { focus },
    };
    try {
      await openNewSession("D:/projects/B");
      expect(store.confirm).toBeNull();
      expect(tabs.some((t) => t.threadId === null)).toBe(true);
      expect(activeTabId.value).not.toBe(SETTINGS_TAB_ID);
      expect(store.panelTab).toBe("history");
      expect(focus).toHaveBeenCalledTimes(1);
    } finally {
      delete (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__;
    }
  });

  it("openHistorySession 成功后：设置标签不再激活、聚焦输入框、切回资源 Tab", async () => {
    openSettingsTab();
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "thread_read") {
        return Promise.resolve({
          thread: { id: "t2", name: "会话2", cwd: "D:/projects/B", turns: [] },
        });
      }
      if (cmd === "codex_rpc") {
        const method = (args as { params?: { method?: string } })?.params
          ?.method;
        if (method === "thread/turns/list") {
          return Promise.resolve({ data: [], nextCursor: null });
        }
      }
      if (cmd === "goal_get") return Promise.resolve({});
      return Promise.resolve(undefined);
    });
    const focus = vi.fn();
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ = {
      commands: { focus },
    };
    try {
      await openHistorySession("t2");
      expect(activeSessionTab()?.threadId).toBe("t2");
      expect(activeSessionTab()?.workspace).toBe("D:/projects/B");
      expect(activeTabId.value).not.toBe(SETTINGS_TAB_ID);
      expect(store.panelTab).toBe("history");
      expect(focus).toHaveBeenCalledTimes(1);
    } finally {
      delete (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__;
    }
  });

  it("openHistorySession：点击当前会话视为已打开，聚焦并切回资源 Tab", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    const focus = vi.fn();
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ = {
      commands: { focus },
    };
    try {
      await openHistorySession("t1");
      expect(focus).toHaveBeenCalledTimes(1);
      expect(store.panelTab).toBe("history");
    } finally {
      delete (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__;
    }
  });
});
describe("新建会话应用记忆模式", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    activeSessionTab()?.attachments.splice(0);
    store.toast = "";
    store.settings.memory_mode = "enabled";
  });

  function mockNewChatFlow() {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_start") {
        return Promise.resolve({ thread: { id: "t-new" } });
      }
      if (cmd === "turn_start") {
        return Promise.resolve({ turn: { id: "nt1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      if (cmd === "codex_rpc") return Promise.resolve({});
      return Promise.resolve(undefined);
    });
  }

  it("新建会话成功后显式应用持久化的记忆模式", async () => {
    tabs.push(makeSessionTab("s-fresh", null));
    activeTabId.value = "s-fresh";
    mockNewChatFlow();
    await sendPrompt("你好");
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/memoryMode/set",
      params: { threadId: "t-new", mode: "enabled" },
    });
    expect(activeSessionTab()?.threadId).toBe("t-new");
  });

  it("待挂载目标（勾选后首条消息）在创建会话时挂载并置为 active", async () => {
    tabs.push(makeSessionTab("s-fresh", null, { goalText: "预填目标" }));
    activeTabId.value = "s-fresh";
    mockNewChatFlow();
    await sendPrompt("你好");
    expect(mockedInvoke).toHaveBeenCalledWith("goal_set", {
      threadId: "t-new",
      objective: "预填目标",
    });
    expect(activeSessionTab()?.goalStatus).toBe("active");
  });

  it("待挂载目标挂载失败：清空本地目标状态并 toast，不阻塞新建", async () => {
    tabs.push(makeSessionTab("s-fresh", null, { goalText: "预填目标" }));
    activeTabId.value = "s-fresh";
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_start") {
        return Promise.resolve({ thread: { id: "t-new" } });
      }
      if (cmd === "turn_start") {
        return Promise.resolve({ turn: { id: "nt1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      if (cmd === "goal_set") return Promise.reject(new Error("挂载失败"));
      if (cmd === "codex_rpc") return Promise.resolve({});
      return Promise.resolve(undefined);
    });
    await sendPrompt("你好");
    expect(activeSessionTab()?.threadId).toBe("t-new");
    expect(activeSessionTab()?.goalText).toBeNull();
    expect(activeSessionTab()?.goalStatus).toBeNull();
    expect(store.toast).toContain("挂载失败");
  });

  it("记忆模式关闭时也显式调用（保证确定性）", async () => {
    store.settings.memory_mode = "disabled";
    mockNewChatFlow();
    await sendPrompt("你好");
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/memoryMode/set",
      params: { threadId: "t-new", mode: "disabled" },
    });
  });

  it("记忆同步失败静默不打扰新建流程", async () => {
    tabs.push(makeSessionTab("s-fresh", null));
    activeTabId.value = "s-fresh";
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_start") {
        return Promise.resolve({ thread: { id: "t-new" } });
      }
      if (cmd === "turn_start") {
        return Promise.resolve({ turn: { id: "nt1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      if (cmd === "codex_rpc") return Promise.reject(new Error("记忆不可用"));
      return Promise.resolve(undefined);
    });
    await sendPrompt("你好");
    expect(activeSessionTab()?.threadId).toBe("t-new");
    expect(store.toast).not.toContain("记忆");
  });
});

describe("模型不可用时的发送路径", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.workspace = null;
    store.interactions = [];
    store.threads = [];
    store.models = [];
    store.toast = "";
  });

  it("无可用模型时发送中止：toast 提示且不调用 turn_start、不落用户消息", async () => {
    tabs.push(makeSessionTab("s1", "t1", { resumedThreadId: "t1" }));
    activeTabId.value = "s1";
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "turn_start") {
        return Promise.resolve({ turn: { id: "nt1" } });
      }
      return Promise.resolve(undefined);
    });

    await sendPrompt("你好");

    expect(store.toast).toContain("当前没有可用模型");
    expect(mockedInvoke).not.toHaveBeenCalledWith("turn_start", expect.anything());
    expect(activeSessionTab()?.turnActive).toBe(false);
    expect(store.itemsByThread["t1"] ?? []).toHaveLength(0);
  });
});

describe("会话级插件/技能缓存生命周期", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.workspace = null;
    store.interactions = [];
    store.threads = [];
  });

  it("newEmptyChat 创建标签即触发 plugin/list 与 skills/list 并写入标签缓存", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "codex_rpc") {
        const method =
          (args as { method?: string } | undefined)?.method ??
          (args as { params?: { method?: string } } | undefined)?.params
            ?.method;
        if (method === "plugin/list") return Promise.resolve(PLUGINS_RESPONSE);
        if (method === "skills/list") return Promise.resolve(SKILLS_RESPONSE);
      }
      return Promise.resolve(undefined);
    });

    await newEmptyChat();

    await vi.waitFor(
      () => {
        expect(activeSessionTab()?.plugins.loaded).toBe(true);
        expect(activeSessionTab()?.skills.loaded).toBe(true);
      },
      { timeout: 3000, interval: 20 },
    );
    expect(activeSessionTab()?.plugins.plugins.length).toBeGreaterThan(0);
    expect(activeSessionTab()?.skills.skills.length).toBeGreaterThan(0);
  });
});

describe("会话创建中标记（creatingChat）", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.workspace = null;
    store.interactions = [];
    store.threads = [];
  });

  it("newChat 创建期间标记发起标签，切换标签后按原标签清回", async () => {
    const tabA = makeSessionTab("sA", null);
    tabs.push(tabA);
    activeTabId.value = "sA";
    let resolveThread!: (v: unknown) => void;
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "thread_start") {
        return new Promise((r) => {
          resolveThread = r;
        });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      if (cmd === "codex_rpc") {
        const method =
          (args as { method?: string } | undefined)?.method ??
          (args as { params?: { method?: string } } | undefined)?.params
            ?.method;
        if (method === "thread/memoryMode/set") return Promise.resolve({});
        return Promise.resolve({});
      }
      return Promise.resolve(undefined);
    });

    const p = sendPrompt("你好");
    // thread_start 挂起期间：发起标签处于「创建中」
    expect(tabA.creatingChat).toBe(true);

    // 创建期间切到另一标签：结果写回原标签，标记也按原标签清回
    const tabB = makeSessionTab("sB", null);
    tabs.push(tabB);
    activeTabId.value = "sB";

    resolveThread({ thread: { id: "t-new" } });
    await p;

    expect(tabA.creatingChat).toBe(false);
    expect(tabB.creatingChat).toBe(false);
    expect(tabA.threadId).toBe("t-new");
  });
});
