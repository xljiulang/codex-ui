import { deleteThread, newEmptyChat, openHistorySession, openNewSession, pickAndOpenNewSession, sendPrompt } from "../useCodex/actions";
import { __resetSessionTabsForTest } from "../useCodex/sessionState";
import { store } from "../useCodex/store";
import { activeTabId } from "../useEditorTabs";
import { makeSessionTab, resetUseCodexState, tabs } from "./useCodexTestHarness";
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

describe("主窗口标题固定为 Codex UI，会话标签标题沿用主窗体格式", () => {
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
    store.currentThreadId = null;
    store.currentThreadName = "";
    store.currentThreadWorkspace = null;
    store.newChatWorkspace = null;
    store.currentThreadOrigin = null;
    store.threads = [];
    store.turnActive = false;
    store.turnInterrupted = false;
    store.currentTurnId = null;
    store.resumedThreadId = null;
    store.threadTokenUsage = null;
    store.goalText = null;
    store.goalStatus = null;
    store.threadPlugins = {};
  });


  it("新建会话不再调用 setTitle（主窗口标题固定）", async () => {
    await newEmptyChat("D:/projects/B");
    expect(store.newChatWorkspace).toBe("D:/projects/B");
    expect(mockWin.setTitle).not.toHaveBeenCalled();
  });
});
describe("主窗口标题固定为 Codex UI，会话标签标题沿用主窗体格式", () => {
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
    store.currentThreadId = null;
    store.currentThreadName = "";
    store.currentThreadWorkspace = null;
    store.newChatWorkspace = null;
    store.currentThreadOrigin = null;
    store.threads = [];
    store.turnActive = false;
    store.turnInterrupted = false;
    store.currentTurnId = null;
    store.resumedThreadId = null;
    store.threadTokenUsage = null;
    store.goalText = null;
    store.goalStatus = null;
    store.threadPlugins = {};
  });


  it("切换会话标签不调用 setTitle", async () => {
    await newEmptyChat();
    const first = activeTabId.value;
    await newEmptyChat("D:/projects/B");
    expect(activeTabId.value).not.toBe(first);
    expect(mockWin.setTitle).not.toHaveBeenCalled();
  });
});
describe("队列模式下发送提示", () => {
  beforeEach(() => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.settings.followup_mode = "queue";
    store.toast = "";
    store.followupQueue.splice(0);
    store.attachments.splice(0);
  });

  it("回合进行中且为队列模式：消息入队并提示", async () => {
    const before = store.userSendRev;
    await sendPrompt("第二条消息");
    // 手动发送标记递增（ChatView 据此强制吸底）；队列落地时不走 sendPrompt，不递增
    expect(store.userSendRev).toBe(before + 1);
    expect(store.followupQueue).toHaveLength(1);
    expect(store.followupQueue[0].text).toBe("第二条消息");
    expect(store.toast).toContain("已加入队列");
  });
});
describe("会话标签状态与事件路由", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.workspace = null;
    store.currentThreadId = null;
    store.currentThreadName = "";
    store.currentThreadWorkspace = null;
    store.newChatWorkspace = null;
    store.turnActive = false;
    store.currentTurnId = null;
    store.goalText = null;
    store.goalStatus = null;
    store.planPrompt = null;
    store.followupQueue = [];
    store.attachments = [];
    store.loading = false;
    store.interactions = [];
    store.threads = [];
  });


  it("新建会话发送首条消息：标题先取消息内容并标记首条消息名", async () => {
    tabs.push(
      makeSessionTab("s1", null, { newChatWorkspace: "D:/repo" }),
    );
    activeTabId.value = "s1";
    store.currentThreadId = null;
    store.currentThreadName = "";
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
    expect(store.currentThreadId).toBe("t1");
    expect(store.currentThreadName).toBe("帮我修复登录页面报错");
    expect(tabs[0].name).toBe("帮我修复登录页面报错");
    expect(tabs[0].nameIsFirstMessage).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("thread_set_name", {
      threadId: "t1",
      name: "帮我修复登录页面报错",
    });
  });
});
describe("会话标签状态与事件路由", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.workspace = null;
    store.currentThreadId = null;
    store.currentThreadName = "";
    store.currentThreadWorkspace = null;
    store.newChatWorkspace = null;
    store.turnActive = false;
    store.currentTurnId = null;
    store.goalText = null;
    store.goalStatus = null;
    store.planPrompt = null;
    store.followupQueue = [];
    store.attachments = [];
    store.loading = false;
    store.interactions = [];
    store.threads = [];
  });


  it("deleteThread：删除唯一标签后允许 0 标签并复位 live 字段", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    store.currentThreadId = "t1";
    mockedInvoke.mockResolvedValue(undefined);
    await deleteThread("t1");
    expect(tabs).toHaveLength(0);
    expect(activeTabId.value).toBe("");
    expect(store.currentThreadId).toBeNull();
  });
});
describe("会话标签状态与事件路由", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.workspace = null;
    store.currentThreadId = null;
    store.currentThreadName = "";
    store.currentThreadWorkspace = null;
    store.newChatWorkspace = null;
    store.turnActive = false;
    store.currentTurnId = null;
    store.goalText = null;
    store.goalStatus = null;
    store.planPrompt = null;
    store.followupQueue = [];
    store.attachments = [];
    store.loading = false;
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
    store.currentThreadId = null;
    store.currentThreadName = "";
    store.currentThreadWorkspace = null;
    store.newChatWorkspace = null;
    store.turnActive = false;
    store.currentTurnId = null;
    store.goalText = null;
    store.goalStatus = null;
    store.planPrompt = null;
    store.followupQueue = [];
    store.attachments = [];
    store.loading = false;
    store.interactions = [];
    store.threads = [];
  });


  it("pickAndOpenNewSession：有会话时初始目录为线程 cwd", async () => {
    store.server.startupWorkspace = "D:/repo";
    store.currentThreadId = "t1";
    store.currentThreadWorkspace = "D:/session";
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
    store.currentThreadId = null;
    store.currentThreadName = "";
    store.currentThreadWorkspace = null;
    store.newChatWorkspace = null;
    store.turnActive = false;
    store.currentTurnId = null;
    store.goalText = null;
    store.goalStatus = null;
    store.planPrompt = null;
    store.followupQueue = [];
    store.attachments = [];
    store.loading = false;
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
    store.currentThreadId = null;
    store.currentThreadName = "";
    store.currentThreadWorkspace = null;
    store.newChatWorkspace = null;
    store.turnActive = false;
    store.currentTurnId = null;
    store.goalText = null;
    store.goalStatus = null;
    store.planPrompt = null;
    store.followupQueue = [];
    store.attachments = [];
    store.loading = false;
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
    store.threadPlugins = {};
    store.turnActive = false;
    store.turnInterrupted = false;
    store.currentThreadId = null;
    store.currentThreadName = "";
    store.currentTurnId = null;
    store.currentThreadOrigin = null;
    store.currentThreadWorkspace = null;
    store.resumedThreadId = null;
    store.threadTokenUsage = null;
    store.goalText = null;
    store.goalStatus = null;
    store.taskMode = "execute";
    store.confirm = null;
    store.showSettings = false;
    store.panelTab = "history";
    store.newChatWorkspace = null;
  });

  it("openNewSession 成功后：关设置页、聚焦输入框、切回资源 Tab", async () => {
    store.showSettings = true;
    const focus = vi.fn();
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ = {
      commands: { focus },
    };
    try {
      await openNewSession("D:/projects/B");
      expect(store.newChatWorkspace).toBe("D:/projects/B");
      expect(store.showSettings).toBe(false);
      expect(store.panelTab).toBe("history");
      expect(focus).toHaveBeenCalledTimes(1);
    } finally {
      delete (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__;
    }
  });

  it("openNewSession：进行中会话不弹确认（会话多开），直接新建标签并收尾", async () => {
    store.showSettings = true;
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    const focus = vi.fn();
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ = {
      commands: { focus },
    };
    try {
      await openNewSession("D:/projects/B");
      expect(store.confirm).toBeNull();
      expect(tabs.some((t) => t.threadId === null)).toBe(true);
      expect(store.showSettings).toBe(false);
      expect(store.panelTab).toBe("history");
      expect(focus).toHaveBeenCalledTimes(1);
    } finally {
      delete (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__;
    }
  });

  it("openHistorySession 成功后：关设置页、聚焦输入框、切回资源 Tab", async () => {
    store.showSettings = true;
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
      expect(store.currentThreadId).toBe("t2");
      expect(store.currentThreadWorkspace).toBe("D:/projects/B");
      expect(store.showSettings).toBe(false);
      expect(store.panelTab).toBe("history");
      expect(focus).toHaveBeenCalledTimes(1);
    } finally {
      delete (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__;
    }
  });

  it("openHistorySession：点击当前会话视为已打开，聚焦并切回资源 Tab", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    store.currentThreadId = "t1";
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
    store.currentThreadId = null;
    store.attachments.splice(0);
    store.toast = "";
    store.settings.memory_mode = "enabled";
    store.taskMode = "execute";
    store.goalText = null;
    store.goalStatus = null;
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
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve(null);
      }
      if (cmd === "codex_rpc") return Promise.resolve({});
      return Promise.resolve(undefined);
    });
  }

  it("新建会话成功后显式应用持久化的记忆模式", async () => {
    mockNewChatFlow();
    await sendPrompt("你好");
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/memoryMode/set",
      params: { threadId: "t-new", mode: "enabled" },
    });
    expect(store.currentThreadId).toBe("t-new");
  });

  it("待挂载目标（勾选后首条消息）在创建会话时挂载并置为 active", async () => {
    store.goalText = "预填目标";
    store.goalStatus = null;
    mockNewChatFlow();
    await sendPrompt("你好");
    expect(mockedInvoke).toHaveBeenCalledWith("goal_set", {
      threadId: "t-new",
      objective: "预填目标",
    });
    expect(store.goalStatus).toBe("active");
  });

  it("待挂载目标挂载失败：清空本地目标状态并 toast，不阻塞新建", async () => {
    store.goalText = "预填目标";
    store.goalStatus = null;
    store.goalArmed = false;
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
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve(null);
      }
      if (cmd === "goal_set") return Promise.reject(new Error("挂载失败"));
      if (cmd === "codex_rpc") return Promise.resolve({});
      return Promise.resolve(undefined);
    });
    await sendPrompt("你好");
    expect(store.currentThreadId).toBe("t-new");
    expect(store.goalText).toBeNull();
    expect(store.goalStatus).toBeNull();
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
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve(null);
      }
      if (cmd === "codex_rpc") return Promise.reject(new Error("记忆不可用"));
      return Promise.resolve(undefined);
    });
    await sendPrompt("你好");
    expect(store.currentThreadId).toBe("t-new");
    expect(store.toast).not.toContain("记忆");
  });
});
