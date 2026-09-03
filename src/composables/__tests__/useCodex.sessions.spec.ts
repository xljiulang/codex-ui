import type { UserInput } from "../../lib/types";
import { newEmptyChat, openThread } from "../useCodex/actions";
import { settleConfirm } from "../useCodex/confirm";
import { __resetSessionTabsForTest, activeSessionTab, isThreadOpen, isThreadRunning, sessionTabTitle } from "../useCodex/sessionState";
import { addAttachmentToActiveSession, closeAllSessionTabs, closeSessionTab, registerComposerAddHandler, switchSessionTab, unregisterComposerAddHandler } from "../useCodex/sessionTabs";
import { store } from "../useCodex/store";
import { continueTurnForTab, interrupt } from "../useCodex/turnControl";
import type { SessionTab } from "../useCodex/types";
import { tabs as _tabs, activeTabId } from "../useEditorTabs";
import { __resetTabsForTest, type Tab } from "../useTabs";
import { makeSessionTab, resetUseCodexState, tabs } from "./useCodexTestHarness";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { flushPromises } from "@vue/test-utils";
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
      codexPath: null,
      logs: [],
    };
    store.threads = [];
  });


  it("会话标签标题：会话标题", () => {
    store.threads = [
      { id: "t1", name: null, preview: "", createdAt: 0, recencyAt: 0 },
    ];
    const tab: SessionTab = {
      id: "s1",
      kind: "chat",
      title: "",
      icon: "chat",
      threadId: "t1",
      name: "我的标题",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      collaborationMode: "default",
      model: null,
      effort: null,
      plugins: { plugins: [], loaded: false },
      skills: { skills: [], loaded: false },
      creatingChat: false,
      draftJson: JSON.stringify({ type: "doc", content: [] }),
      draftAttachments: [],
      draftRefs: {},
      origin: "session",
      workspace: "D:/repo/sub",
      resumedThreadId: null,
      turnActive: false,
      currentTurnId: null,
      turnInterrupted: false,
      goalText: null,
      goalStatus: null,
      goalArmed: false,
      threadTokenUsage: null,
      followupQueue: [],
      attachments: [],
      planPrompt: null,
      plan: null,
      loading: false,
      newChatWorkspace: null,
      interactions: [],
    };
    expect(sessionTabTitle(tab)).toBe("我的标题");
  });
});


describe("打开历史会话即恢复（token 用量显示）", () => {
  beforeEach(() => {
    store.server = {
      connected: false,
      codexPath: null,
      logs: [],
    };
  });

  function baseMock(
    goalResponse: Record<string, unknown>,
    resumeError?: string,
  ) {
    return (cmd: string, args?: unknown) => {
      if (cmd === "thread_read") {
        return Promise.resolve({ thread: { id: "t2", name: null, turns: [] } });
      }
      if (cmd === "codex_rpc") {
        const method = (args as { params?: { method?: string } })?.params
          ?.method;
        if (method === "thread/turns/list") {
          return Promise.resolve({ data: [], nextCursor: null });
        }
      }
      if (cmd === "goal_get") return Promise.resolve(goalResponse);
      if (cmd === "thread_resume")
        return resumeError
          ? Promise.reject(new Error(resumeError))
          : Promise.resolve({});
      return Promise.resolve(undefined);
    };
  }

  it("无目标：打开即调用 thread_resume 并置 resumedThreadId", async () => {
    store.threads = [];
    mockedInvoke.mockImplementation(baseMock({}));
    expect(await openThread("t2")).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("thread_resume", {
      params: { threadId: "t2" },
    });
    expect(activeSessionTab()?.resumedThreadId).toBe("t2");
  });

  it("带活跃目标：打开不调用 thread_resume，resumedThreadId 保持 null", async () => {
    store.threads = [];
    mockedInvoke.mockImplementation(
      baseMock({ goal: { objective: "改代码", status: "active" } }),
    );
    expect(await openThread("t2")).toBe(true);
    const resumeCalls = mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "thread_resume",
    );
    expect(resumeCalls).toHaveLength(0);
    expect(activeSessionTab()?.resumedThreadId).toBeNull();
  });

  it("带终态目标：目标被清后仍调用 thread_resume", async () => {
    store.threads = [];
    mockedInvoke.mockImplementation(
      baseMock({ goal: { objective: "改代码", status: "complete" } }),
    );
    expect(await openThread("t2")).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("thread_resume", {
      params: { threadId: "t2" },
    });
    expect(activeSessionTab()?.resumedThreadId).toBe("t2");
  });

  it("thread_resume 失败（非 not-found）：仍成功打开、不丢弃标签", async () => {
    store.threads = [];
    mockedInvoke.mockImplementation(
      baseMock({}, "boom"),
    );
    expect(await openThread("t2")).toBe(true);
    expect(tabs).toHaveLength(1);
    expect(activeSessionTab()?.threadId).toBe("t2");
    expect(activeSessionTab()?.resumedThreadId).toBeNull();
  });
});
describe("主窗口标题固定为 Codex UI，会话标签标题沿用主窗体格式", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockWin.setTitle.mockClear();
    __resetSessionTabsForTest();
    store.server = {
      connected: false,
      codexPath: null,
      logs: [],
    };
    store.threads = [];
  });


  it("会话标签标题：无标题回退摘要预览", () => {
    store.threads = [
      { id: "t1", name: null, preview: "预览文本", createdAt: 0, recencyAt: 0 },
    ];
    const tab: SessionTab = {
      id: "s1",
      kind: "chat",
      title: "",
      icon: "chat",
      threadId: "t1",
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      collaborationMode: "default",
      model: null,
      effort: null,
      plugins: { plugins: [], loaded: false },
      skills: { skills: [], loaded: false },
      creatingChat: false,
      draftJson: JSON.stringify({ type: "doc", content: [] }),
      draftAttachments: [],
      draftRefs: {},
      origin: "session",
      workspace: "D:/repo",
      resumedThreadId: null,
      turnActive: false,
      currentTurnId: null,
      turnInterrupted: false,
      goalText: null,
      goalStatus: null,
      goalArmed: false,
      threadTokenUsage: null,
      followupQueue: [],
      attachments: [],
      planPrompt: null,
      plan: null,
      loading: false,
      newChatWorkspace: null,
      interactions: [],
    };
    expect(sessionTabTitle(tab)).toBe("预览文本");
  });
});
describe("主窗口标题固定为 Codex UI，会话标签标题沿用主窗体格式", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockWin.setTitle.mockClear();
    __resetSessionTabsForTest();
    store.server = {
      connected: false,
      codexPath: null,
      logs: [],
    };
    store.threads = [];
  });


  it("会话标签标题：新建带目录仅名称", () => {
    const tab: SessionTab = {
      id: "s1",
      kind: "chat",
      title: "",
      icon: "chat",
      threadId: null,
      name: "标题",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      collaborationMode: "default",
      model: null,
      effort: null,
      plugins: { plugins: [], loaded: false },
      skills: { skills: [], loaded: false },
      creatingChat: false,
      draftJson: JSON.stringify({ type: "doc", content: [] }),
      draftAttachments: [],
      draftRefs: {},
      origin: null,
      workspace: null,
      resumedThreadId: null,
      turnActive: false,
      currentTurnId: null,
      turnInterrupted: false,
      goalText: null,
      goalStatus: null,
      goalArmed: false,
      threadTokenUsage: null,
      followupQueue: [],
      attachments: [],
      planPrompt: null,
      plan: null,
      loading: false,
      newChatWorkspace: "D:/repo",
      interactions: [],
    };
    expect(sessionTabTitle(tab)).toBe("标题");
  });
});
describe("主窗口标题固定为 Codex UI，会话标签标题沿用主窗体格式", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockWin.setTitle.mockClear();
    __resetSessionTabsForTest();
    store.server = {
      connected: false,
      codexPath: null,
      logs: [],
    };
    store.threads = [];
  });


  it("会话标签标题：全新标签为 新建会话", () => {
    const tab: SessionTab = {
      id: "s1",
      kind: "chat",
      title: "",
      icon: "chat",
      threadId: null,
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      collaborationMode: "default",
      model: null,
      effort: null,
      plugins: { plugins: [], loaded: false },
      skills: { skills: [], loaded: false },
      creatingChat: false,
      draftJson: JSON.stringify({ type: "doc", content: [] }),
      draftAttachments: [],
      draftRefs: {},
      origin: null,
      workspace: null,
      resumedThreadId: null,
      turnActive: false,
      currentTurnId: null,
      turnInterrupted: false,
      goalText: null,
      goalStatus: null,
      goalArmed: false,
      threadTokenUsage: null,
      followupQueue: [],
      attachments: [],
      planPrompt: null,
      plan: null,
      loading: false,
      newChatWorkspace: "D:/repo",
      interactions: [],
    };
    expect(sessionTabTitle(tab)).toBe("新建会话");
  });
});
describe("主窗口标题固定为 Codex UI，会话标签标题沿用主窗体格式", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockWin.setTitle.mockClear();
    __resetSessionTabsForTest();
    store.server = {
      connected: false,
      codexPath: null,
      logs: [],
    };
    store.threads = [];
  });


  it("会话标签标题：新对话预选目录后为 新建会话", () => {
    const tab: SessionTab = {
      id: "s1",
      kind: "chat",
      title: "",
      icon: "chat",
      threadId: null,
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      collaborationMode: "default",
      model: null,
      effort: null,
      plugins: { plugins: [], loaded: false },
      skills: { skills: [], loaded: false },
      creatingChat: false,
      draftJson: JSON.stringify({ type: "doc", content: [] }),
      draftAttachments: [],
      draftRefs: {},
      origin: null,
      workspace: null,
      resumedThreadId: null,
      turnActive: false,
      currentTurnId: null,
      turnInterrupted: false,
      goalText: null,
      goalStatus: null,
      goalArmed: false,
      threadTokenUsage: null,
      followupQueue: [],
      attachments: [],
      planPrompt: null,
      plan: null,
      loading: false,
      newChatWorkspace: "D:/projects/B",
      interactions: [],
    };
    expect(sessionTabTitle(tab)).toBe("新建会话");
  });
});
describe("主窗口标题固定为 Codex UI，会话标签标题沿用主窗体格式", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockWin.setTitle.mockClear();
    __resetSessionTabsForTest();
    store.server = {
      connected: false,
      codexPath: null,
      logs: [],
    };
    store.threads = [];
  });


  it("会话标签标题：无 workspace 且无 cwd 时降级为仅标题", () => {
    store.workspace = "";
    const tab: SessionTab = {
      id: "s1",
      kind: "chat",
      title: "",
      icon: "chat",
      threadId: null,
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      collaborationMode: "default",
      model: null,
      effort: null,
      plugins: { plugins: [], loaded: false },
      skills: { skills: [], loaded: false },
      creatingChat: false,
      draftJson: JSON.stringify({ type: "doc", content: [] }),
      draftAttachments: [],
      draftRefs: {},
      origin: null,
      workspace: null,
      resumedThreadId: null,
      turnActive: false,
      currentTurnId: null,
      turnInterrupted: false,
      goalText: null,
      goalStatus: null,
      goalArmed: false,
      threadTokenUsage: null,
      followupQueue: [],
      attachments: [],
      planPrompt: null,
      plan: null,
      loading: false,
      newChatWorkspace: null,
      interactions: [],
    };
    expect(sessionTabTitle(tab)).toBe("新建会话");
  });
});
describe("多会话标签：新建/打开/切换/关闭", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.confirm = null;
  });

  it("新建会话：创建新标签且不中断旧回合（会话多开）", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", { turnActive: true, currentTurnId: "turn-1" }),
    );
    activeTabId.value = "s1";

    expect(await newEmptyChat()).toBe(true);
    expect(store.confirm).toBeNull();
    expect(tabs).toHaveLength(2);
    expect(activeTabId.value).toBe(tabs[1].id);
    expect(activeSessionTab()?.threadId).toBeNull();
    expect(activeSessionTab()?.currentTurnId).toBeNull();
    const interruptCalls = mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "turn_interrupt",
    );
    expect(interruptCalls).toHaveLength(0);
    // 旧标签保持进行中（后台继续运行）
    expect(tabs[0].turnActive).toBe(true);
    expect(tabs[0].currentTurnId).toBe("turn-1");
  });

  it("切换到历史会话：不中断当前回合，新建标签并加载", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", { turnActive: true, currentTurnId: "turn-1" }),
    );
    activeTabId.value = "s1";
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "thread_read") {
        return Promise.resolve({
          thread: { id: "t2", name: "会话2", turns: [] },
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

    expect(await openThread("t2")).toBe(true);
    expect(store.confirm).toBeNull();
    const interruptCalls = mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "turn_interrupt",
    );
    expect(interruptCalls).toHaveLength(0);
    expect(tabs).toHaveLength(2);
    expect(activeSessionTab()?.threadId).toBe("t2");
    expect(tabs[0].turnActive).toBe(true);
  });

  it("点击当前正在进行的会话（同线程）：不算切换，不中断、不重载", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", { turnActive: true, currentTurnId: "turn-1" }),
    );
    activeTabId.value = "s1";
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_read") {
        return Promise.resolve({
          thread: { id: "t1", name: "会话1", turns: [] },
        });
      }
      return Promise.resolve(undefined);
    });

    expect(await openThread("t1")).toBe(true);
    const interruptCalls = mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "turn_interrupt",
    );
    expect(interruptCalls).toHaveLength(0);
    expect(store.confirm).toBeNull();
    expect(activeSessionTab()?.turnActive).toBe(true);
    expect(activeSessionTab()?.currentTurnId).toBe("turn-1");
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "thread_read",
      expect.anything(),
    );
    expect(tabs).toHaveLength(1);
  });

  it("同一会话已打开：重复打开只聚焦已有标签，不新建、不重载", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    tabs.push(makeSessionTab("s2", "t2"));
    activeTabId.value = "s1";
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_read") {
        return Promise.resolve({
          thread: { id: "t2", name: "会话2", turns: [] },
        });
      }
      return Promise.resolve(undefined);
    });

    expect(await openThread("t2")).toBe(true);
    expect(activeTabId.value).toBe("s2");
    expect(activeSessionTab()?.threadId).toBe("t2");
    expect(tabs).toHaveLength(2);
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "thread_read",
      expect.anything(),
    );
  });

  it("关闭运行中的会话标签：先确认，确认后中断并移除", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", { turnActive: true, currentTurnId: "turn-1" }),
    );
    activeTabId.value = "s1";

    const p = closeSessionTab("s1");
    expect(store.confirm?.title).toBe("关闭会话标签");
    settleConfirm(true);
    await p;
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(tabs.some((t) => t.id === "s1")).toBe(false);
    // 允许 0 个会话标签：不兜底新建，live 字段复位到无会话默认态
    expect(tabs).toHaveLength(0);
    expect(activeTabId.value).toBe("");
    expect(activeSessionTab()).toBeNull();
  });

  it("关闭运行中的会话标签：取消确认则保留、不中断", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", { turnActive: true, currentTurnId: "turn-1" }),
    );
    activeTabId.value = "s1";

    const p = closeSessionTab("s1");
    expect(store.confirm).not.toBeNull();
    settleConfirm(false);
    await p;
    expect(tabs.some((t) => t.id === "s1")).toBe(true);
    const interruptCalls = mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "turn_interrupt",
    );
    expect(interruptCalls).toHaveLength(0);
  });

  it("有活跃目标时标准停止：先清目标再 turn/interrupt（默认操作当前会话）", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", {
        turnActive: true,
        currentTurnId: "turn-1",
        goalText: "目标",
        goalStatus: "active",
        goalArmed: true,
      }),
    );
    activeTabId.value = "s1";

    await interrupt();
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", {
      threadId: "t1",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(activeSessionTab()?.goalText).toBeNull();
    expect(activeSessionTab()?.goalStatus).toBeNull();
    expect(activeSessionTab()?.goalArmed).toBe(false);
    expect(tabs[0].goalText).toBeNull();
  });

  it("显式传入旧线程/回合 id 时：目标清除作用于旧线程，且不污染新会话的回合 id", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", {
        goalText: "旧目标",
        goalStatus: "active",
      }),
    );
    tabs.push(
      makeSessionTab("s2", "t2", {
        goalText: "旧目标",
        goalStatus: "active",
      }),
    );
    activeTabId.value = "s2";
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "turn_interrupt") {
        const turnId = (args as { turnId?: string })?.turnId;
        if (turnId === "turn-1") {
          return Promise.reject(
            new Error("turn not found but found abc-123"),
          );
        }
      }
      return Promise.resolve(undefined);
    });
    // 显式中断旧线程 t1（无标签）：目标清除作用于 t1，本地展示（当前线程 t2）不被清空
    await interrupt("t1", "turn-1");
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", {
      threadId: "t1",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "abc-123",
    });
    expect(activeSessionTab()?.currentTurnId).toBeNull();
    expect(activeSessionTab()?.goalText).toBe("旧目标");
    expect(activeSessionTab()?.goalStatus).toBe("active");
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


  it("会话标签 title 由同步点维护：新建为 新建会话，改名后更新", async () => {
    await newEmptyChat("D:/projects/B");
    expect(tabs[0].title).toBe("新建会话");

    await newEmptyChat("D:/repo");
    const tab = tabs[1];
    expect(tab.title).toBe("新建会话");
    tab.name = "我的标题";
    tab.newChatWorkspace = "D:/projects/B/sub";
    tab.title = sessionTabTitle(tab);
    expect(tab.title).toBe("我的标题");
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


  it("会话标签 loading 字段读写一致", async () => {
    await newEmptyChat();
    const tab = tabs[0];
    expect(tab.loading).toBe(false);
    tab.loading = true;
    expect(tab.loading).toBe(true);
    tab.loading = false;
    expect(tab.loading).toBe(false);
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


  it("isThreadOpen / isThreadRunning 反映标签打开与运行状态", () => {
    tabs.push(makeSessionTab("s1", "t1", { turnActive: true }));
    tabs.push(makeSessionTab("s2", "t2"));
    expect(isThreadOpen("t1")).toBe(true);
    expect(isThreadOpen("t2")).toBe(true);
    expect(isThreadOpen("t3")).toBe(false);
    expect(isThreadRunning("t1")).toBe(true);
    expect(isThreadRunning("t2")).toBe(false);
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


  it("switchSessionTab：快照当前、恢复目标，各标签状态不串", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", {
        permissionMode: "full-access",
        collaborationMode: "plan",
        model: "gpt-5",
        effort: "high",
        turnActive: true,
        currentTurnId: "turn-1",
        goalText: "目标A",
        goalStatus: "active",
        attachments: [{ type: "text", text: "草稿A", text_elements: [] }],
        followupQueue: [{ text: "队列A", attachments: [] }],
      }),
    );
    tabs.push(
      makeSessionTab("s2", "t2", {
        name: "会话B",
        workspace: "D:/repo/b",
        planPrompt: { threadId: "t2", turnId: "tp2", planText: "计划B" },
        permissionMode: "help-me-approve",
        collaborationMode: "default",
        model: "o3",
        effort: "low",
      }),
    );
    activeTabId.value = "s1";
    await flushPromises(); // 活动标签记录与 live 字段同步

    expect(await switchSessionTab("s2")).toBe(true);
    expect(activeTabId.value).toBe("s2");
    expect(activeSessionTab()?.threadId).toBe("t2");
    expect(activeSessionTab()?.name).toBe("会话B");
    expect(activeSessionTab()?.workspace).toBe("D:/repo/b");
    expect(activeSessionTab()?.turnActive).toBe(false);
    expect(activeSessionTab()?.planPrompt?.planText).toBe("计划B");
    expect(activeSessionTab()?.permissionMode).toBe("help-me-approve");
    expect(activeSessionTab()?.collaborationMode).toBe("default");
    expect(activeSessionTab()?.model).toBe("o3");
    expect(activeSessionTab()?.effort).toBe("low");

    expect(await switchSessionTab("s1")).toBe(true);
    expect(activeSessionTab()?.threadId).toBe("t1");
    expect(activeSessionTab()?.turnActive).toBe(true);
    expect(activeSessionTab()?.currentTurnId).toBe("turn-1");
    expect(activeSessionTab()?.goalText).toBe("目标A");
    expect(activeSessionTab()?.attachments).toHaveLength(1);
    expect(activeSessionTab()?.followupQueue).toHaveLength(1);
    expect(activeSessionTab()?.permissionMode).toBe("full-access");
    expect(activeSessionTab()?.collaborationMode).toBe("plan");
    expect(activeSessionTab()?.model).toBe("gpt-5");
    expect(activeSessionTab()?.effort).toBe("high");
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


  it("活动标签的 live 字段变化自动同步回标签记录", async () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "会话A", collaborationMode: "plan" }));
    activeTabId.value = "s1";
    expect(tabs[0].name).toBe("会话A");
    expect(tabs[0].collaborationMode).toBe("plan");
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


  it("closeAllSessionTabs：运行中跳过并计数，关闭后保留运行中标签", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    tabs.push(
      makeSessionTab("s2", "t2", { turnActive: true }),
    );
    activeTabId.value = "s1";
    const skipped = await closeAllSessionTabs();
    expect(skipped).toBe(1);
    expect(tabs.some((t) => t.id === "s1")).toBe(false);
    expect(tabs.some((t) => t.id === "s2")).toBe(true);
    expect(tabs).toHaveLength(1);
    expect(activeTabId.value).toBe(
      tabs[tabs.length - 1].id,
    );
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


  it("closeAllSessionTabs：全部关闭后允许 0 标签", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    const skipped = await closeAllSessionTabs();
    expect(skipped).toBe(0);
    expect(tabs).toHaveLength(0);
    expect(activeTabId.value).toBe("");
  });
});
describe("会话级附件注册表（资源面板 @ 路由）", () => {
  const mention = (name: string, path: string): UserInput => ({
    type: "mention",
    name,
    path,
  });

  beforeEach(() => {
    __resetSessionTabsForTest();
    activeTabId.value = "";
  });

  it("有活动会话且已注册处理器：路由成功并调用处理器", () => {
    tabs.push(makeSessionTab("s1", null));
    activeTabId.value = "s1";
    const fn = vi.fn();
    registerComposerAddHandler("s1", fn);

    const a = mention("a.txt", "D:/repo/a.txt");
    expect(addAttachmentToActiveSession(a)).toBe(true);
    expect(fn).toHaveBeenCalledWith(a);
    unregisterComposerAddHandler("s1");
  });

  it("无活动会话：返回 false 且不调用任何处理器", () => {
    const fn = vi.fn();
    registerComposerAddHandler("s1", fn);

    expect(addAttachmentToActiveSession(mention("a.txt", "p"))).toBe(false);
    expect(fn).not.toHaveBeenCalled();
    unregisterComposerAddHandler("s1");
  });

  it("活动会话未注册处理器：返回 false", () => {
    tabs.push(makeSessionTab("s1", null));
    activeTabId.value = "s1";

    expect(addAttachmentToActiveSession(mention("a.txt", "p"))).toBe(false);
  });

  it("注销后不再路由到该会话", () => {
    tabs.push(makeSessionTab("s1", null));
    activeTabId.value = "s1";
    const fn = vi.fn();
    registerComposerAddHandler("s1", fn);
    unregisterComposerAddHandler("s1");

    expect(addAttachmentToActiveSession(mention("a.txt", "p"))).toBe(false);
    expect(fn).not.toHaveBeenCalled();
  });
});
describe("统一列表：活动会话 live 字段投影", () => {
  /** 最小文件标签 fixture（仅验证活动标签非会话时投影保持） */
  function makeFileTabFixture(id: string): Tab {
    return {
      kind: "file",
      id,
      title: "a.txt",
      icon: "file",
      workspace: "D:/repo",
      path: "a.txt",
      loading: false,
      error: "",
      readOnly: false,
      dirty: false,
      saving: false,
      wrap: false,
      markdownPreview: false,
      eol: "lf",
      hadBom: false,
      byteSize: null,
      cursor: { line: 0, col: 0 },
      status: "",
      editorState: null,
      savedText: null,
      wrapCompartment: null,
    } as unknown as Tab;
  }

  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetTabsForTest();
  });

  it("切换会话标签：live 字段投影为目标标签状态", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", {
        name: "会话一",
        workspace: "D:/a",
        turnActive: true,
      }),
    );
    tabs.push(
      makeSessionTab("s2", "t2", {
        name: "会话二",
        workspace: "D:/b",
        turnActive: false,
      }),
    );

    activeTabId.value = "s1";
    expect(activeSessionTab()?.threadId).toBe("t1");
    expect(activeSessionTab()?.name).toBe("会话一");
    expect(activeSessionTab()?.workspace).toBe("D:/a");
    expect(activeSessionTab()?.turnActive).toBe(true);

    activeTabId.value = "s2";
    expect(activeSessionTab()?.threadId).toBe("t2");
    expect(activeSessionTab()?.name).toBe("会话二");
    expect(activeSessionTab()?.workspace).toBe("D:/b");
    expect(activeSessionTab()?.turnActive).toBe(false);
  });

  it("切到文件标签：live 字段保持最近会话不变，切回后仍一致", () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "会话一" }));
    activeTabId.value = "s1";
    expect(activeSessionTab()?.threadId).toBe("t1");

    (_tabs as unknown as Tab[]).push(makeFileTabFixture("file:test"));
    activeTabId.value = "file:test";
    expect(activeSessionTab()?.threadId).toBe("t1");
    expect(activeSessionTab()?.name).toBe("会话一");

    activeTabId.value = "s1";
    expect(activeSessionTab()?.threadId).toBe("t1");
  });

  it("文件标签活动时点击历史会话：聚焦已打开的会话标签（不新建、不重载）", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    (_tabs as unknown as Tab[]).push(makeFileTabFixture("file:test"));
    activeTabId.value = "file:test";
    // bug 前提：文件标签活动时投影仍指向最近会话 s1，但显示中的标签是文件
    expect(activeSessionTab()?.threadId).toBe("t1");

    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_read") {
        return Promise.resolve({
          thread: { id: "t1", name: "会话1", turns: [] },
        });
      }
      return Promise.resolve(undefined);
    });

    expect(await openThread("t1")).toBe(true);
    // 修复点：即使投影会话就是 s1，点击历史会话也必须把 s1 切回前台
    expect(activeTabId.value).toBe("s1");
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "thread_read",
      expect.anything(),
    );
    expect(tabs).toHaveLength(2); // 会话 + 文件：不新建标签
  });

  it("关闭活动会话：投影切到相邻会话", async () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "会话一" }));
    tabs.push(makeSessionTab("s2", "t2", { name: "会话二" }));
    activeTabId.value = "s1";
    expect(activeSessionTab()?.threadId).toBe("t1");

    await closeSessionTab("s1");
    expect(activeTabId.value).toBe("s2");
    expect(activeSessionTab()?.threadId).toBe("t2");
    expect(activeSessionTab()?.name).toBe("会话二");
  });

  it("关闭最后一个会话：live 复位默认态，无活动标签", async () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "会话一" }));
    activeTabId.value = "s1";

    await closeSessionTab("s1");
    expect(activeTabId.value).toBe("");
    expect(activeSessionTab()).toBeNull();
  });
});

describe("多会话隔离：关闭/发送不触碰其它标签", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.itemsByThread = {};
    store.activeWorkByThread = {};
  });

  it("关闭后台运行中且无回合 id 的标签：不借用活动标签回合 id 发 turn_interrupt", async () => {
    // A 活动：正在跑回合（TA）；B 后台：turnActive 但回合 id 尚未回填
    tabs.push(
      makeSessionTab("sA", "tA", { turnActive: true, currentTurnId: "TA" }),
    );
    tabs.push(
      makeSessionTab("sB", "tB", { turnActive: true, currentTurnId: null }),
    );
    activeTabId.value = "sA";

    const p = closeSessionTab("sB");
    expect(store.confirm?.title).toBe("关闭会话标签");
    settleConfirm(true);
    await p;

    // 目标线程无回合 id：不得借用 store 的 TA 发 turn_interrupt（避免误中断 A）
    const interruptCalls = mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "turn_interrupt",
    );
    expect(interruptCalls).toHaveLength(0);
    expect(tabs.some((t) => t.id === "sB")).toBe(false);
    // A 的回合状态不受影响
    expect(activeSessionTab()?.threadId).toBe("tA");
    expect(activeSessionTab()?.currentTurnId).toBe("TA");
    expect(activeSessionTab()?.turnActive).toBe(true);
  });

  it("后台标签发送使用自己的协作模式：tab=plan、store=default 时 collaborationMode.mode=plan", async () => {
    const tab = makeSessionTab("sB", "tB", {
      collaborationMode: "plan",
      resumedThreadId: "tB",
    });
    tabs.push(tab);
    tab.model = "gpt-5";
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "turn_start") {
        return Promise.resolve({ turn: { id: "turn-b" } });
      }
      return Promise.resolve(undefined);
    });

    await continueTurnForTab(tab, "hello", []);

    const call = mockedInvoke.mock.calls.find(([c]) => c === "turn_start");
    const params = (call?.[1] as { params?: Record<string, unknown> })
      ?.params;
    expect(params?.collaborationMode).toMatchObject({ mode: "plan" });
    expect(tab.turnActive).toBe(true);
    expect(tab.currentTurnId).toBe("turn-b");
  });
});
