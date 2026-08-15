import { describe, expect, it, beforeEach, afterEach } from "vitest";
import { vi } from "vitest";
import { flushPromises } from "@vue/test-utils";

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

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  addAttachmentToActiveSession,
  __resetPinnedSectionForTest,
  __resetTitleHelperCapabilityForTest,
  autoTitleThread,
  clearGoal,
  closeAllSessionTabs,
  closeSessionTab,
  deleteThread,
  dismissPlanPrompt,
  disposeEvents,
  ensureSkills,
  ensureThreadPlugins,
  executePlan,
  exitPlanMode,
  init,
  interrupt,
  isGoalStatus,
  isThreadOpen,
  isThreadRunning,
  loadSettings,
  newEmptyChat,
  openHistorySession,
  openNewSession,
  openThread,
  pickAndOpenNewSession,
  refreshThreads,
  refreshServer,
  registerComposerAddHandler,
  resolveSessionWorkspace,
  sanitizeTitle,
  searchThreads,
  sessionTabTitle,
  sendPrompt,
  setGoal,
  settleConfirm,
  sortThreads,
  store,
  switchSessionTab,
  __resetSessionTabsForTest,
  togglePin,
  toastError,
  unregisterComposerAddHandler,
  wireEvents,
  workspace,
} from "../useCodex";
import type { SessionTab } from "../useCodex";
import type { UserInput } from "../../lib/types";
import { activeTabId, tabs as _tabs } from "../useEditorTabs";
import { __resetTabsForTest, type Tab } from "../useTabs";
/** 统一列表在本 spec 中只放会话标签 fixture，按 SessionTab 数组使用 */
const tabs = _tabs as unknown as SessionTab[];

const mockedInvoke = vi.mocked(invoke);
const mockedListen = vi.mocked(listen);

/** 每个事件名可注册多个回调（wireEvents 与 autoTitleThread 会同时监听） */
const capturedListeners: Record<string, Array<(ev: { payload?: unknown }) => void>> =
  {};

function mockListenCapture() {
  mockedListen.mockImplementation(async (event, cb) => {
    (capturedListeners[event] ??= []).push(
      cb as unknown as (ev: { payload?: unknown }) => void,
    );
    return () => {};
  });
}

function fireListen(event: string, payload: unknown) {
  for (const cb of capturedListeners[event] ?? []) {
    cb({ payload });
  }
}

/** 构造会话标签 fixture（多会话标签测试用） */
function makeSessionTab(
  id: string,
  threadId: string | null,
  over: Partial<SessionTab> = {},
): SessionTab {
  return {
    id,
    threadId,
    name: "",
    nameIsFirstMessage: false,
    permissionMode: "ask-for-approval",
    taskMode: "execute",
    model: null,
    effort: null,
    draftJson: JSON.stringify({ type: "doc", content: [] }),
    draftAttachments: [],
    draftRefs: {},
    origin: threadId ? "history" : null,
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
    loading: false,
    newChatWorkspace: null,
    interactions: [],
    ...over,
    kind: "chat",
    title: "",
    icon: "chat",
  };
}

const SKILLS_RESPONSE = {
  data: [
    {
      skills: [
        {
          name: "csharp-code-rules",
          key: "csharp-code-rules",
          path: "C:/x/skills/csharp-code-rules/SKILL.md",
          description: "C# 代码规范长描述",
          interface: {
            shortDescription: "C# 代码规范短描述",
          },
        },
        {
          name: "disabled-skill",
          key: "disabled-skill",
          path: "C:/x/disabled/SKILL.md",
          desc: "已禁用",
          enabled: false,
        },
      ],
    },
  ],
};

const PLUGINS_RESPONSE = {
  marketplaces: [
    {
      plugins: [
        {
          id: "documents@openai-primary-runtime",
          name: "documents",
          installed: true,
          enabled: true,
          source: { path: "C:/x/documents" },
          interface: {
            displayName: "Documents",
            shortDescription: "文档处理",
            composerIcon: "C:/x/documents/icon.png",
            composerIconUrl: null,
            brandColor: "#2563EB",
          },
        },
        {
          id: "pdf@openai-primary-runtime",
          name: "pdf",
          installed: true,
          enabled: true,
          source: { path: "C:/x/pdf" },
          interface: { displayName: "PDF" },
        },
        {
          id: "disabled@openai-curated",
          name: "disabled",
          installed: false,
          enabled: false,
          source: { path: "C:/x/disabled" },
          interface: { displayName: "Disabled" },
        },
      ],
    },
    {
      plugins: [
        {
          id: "documents@openai-primary-runtime",
          name: "documents",
          installed: true,
          enabled: true,
          source: { path: "C:/x/dup" },
          interface: { displayName: "Duplicate" },
        },
      ],
    },
  ],
};

describe("toastError 错误提示提取", () => {
  it("优先提取 error.error.message / error.message，回退 String(e)", () => {
    expect(toastError({ error: { message: "服务端错误" } })).toBe("服务端错误");
    expect(toastError({ message: "普通错误" })).toBe("普通错误");
    expect(toastError("raw string")).toBe("raw string");
    expect(toastError(42)).toBe("42");
    expect(toastError(null)).toBe("null");
  });

  it("服务端已知消息映射为友好中文", () => {
    expect(
      toastError({
        error: {
          message:
            "Context window exceeded while compacting; removing oldest history item.",
        },
      }),
    ).toBe("上下文超出窗口，已自动压缩并移除最早的历史内容");
    expect(
      toastError("Server overloaded; retry later."),
    ).toBe("服务过载，请稍后重试");
    expect(
      toastError("cannot resume running thread thr_1 with history while it is already running"),
    ).toBe("该会话正被占用（已有回合在运行或其它进程持有），请稍后再试");
  });
});

describe("启动加载态 booting 状态", () => {
  beforeEach(() => {
    disposeEvents(); // 重置 wired，避免 init() 内的 wireEvents 与其它用例互相干扰
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    __resetSessionTabsForTest();
    store.booting = true;
  });

  it("初始为 true，init() 完成后置为 false", async () => {
    expect(store.booting).toBe(true);
    await init();
    expect(store.booting).toBe(false);
    // 允许 0 个会话标签：启动不自动创建
    expect(tabs).toHaveLength(0);
    expect(activeTabId.value).toBe("");
  });

  it("init() 抛错时也关闭加载态", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "server_status") {
        return Promise.reject(new Error("后端不可用"));
      }
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    expect(store.booting).toBe(true);
    await expect(init()).rejects.toThrow("后端不可用");
    expect(store.booting).toBe(false);
  });
});

describe("服务端 error/warning 事件 toast 本地化", () => {
  beforeEach(() => {
    disposeEvents(); // 重置 wired，确保本组用例重新注册监听
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    store.toast = "";
  });

  it("error 事件：thread 占用原文映射为中文 toast", async () => {
    await wireEvents();
    fireListen("error", {
      error: {
        message:
          "cannot resume running thread thr_9 with history while it is already running",
      },
    });
    expect(store.toast).toBe(
      "该会话正被占用（已有回合在运行或其它进程持有），请稍后再试",
    );
  });

  it("error 事件：窗口压缩原文映射为中文 toast", async () => {
    await wireEvents();
    fireListen("error", {
      error: {
        message:
          "Context window exceeded while compacting; removing oldest history item. Error: oops",
      },
    });
    expect(store.toast).toBe(
      "上下文超出窗口，已自动压缩并移除最早的历史内容",
    );
  });

  it("error 事件：codexErrorInfo 结构化映射优先", async () => {
    await wireEvents();
    fireListen("error", {
      error: { message: "raw message", codexErrorInfo: "contextWindowExceeded" },
    });
    expect(store.toast).toBe("上下文已超出模型窗口");
  });

  it("error 事件：无 message 时回退通用文案", async () => {
    await wireEvents();
    fireListen("error", { error: {} });
    expect(store.toast).toBe("codex 发生错误");
  });

  it("warning 事件：原文映射为中文 toast，未匹配保留原文", async () => {
    await wireEvents();
    fireListen("warning", { message: "Server overloaded; retry later." });
    expect(store.toast).toBe("服务过载，请稍后重试");
    fireListen("warning", { message: "some future warning" });
    expect(store.toast).toBe("some future warning");
  });
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

  it("切换会话标签不调用 setTitle", async () => {
    await newEmptyChat();
    const first = activeTabId.value;
    await newEmptyChat("D:/projects/B");
    expect(activeTabId.value).not.toBe(first);
    expect(mockWin.setTitle).not.toHaveBeenCalled();
  });

  it("会话标签标题：目录名 / 会话标题", () => {
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
      taskMode: "execute",
      model: null,
      effort: null,
      draftJson: JSON.stringify({ type: "doc", content: [] }),
      draftAttachments: [],
      draftRefs: {},
      origin: "history",
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
      loading: false,
      newChatWorkspace: null,
      interactions: [],
    };
    expect(sessionTabTitle(tab)).toBe("sub / 我的标题");
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
      taskMode: "execute",
      model: null,
      effort: null,
      draftJson: JSON.stringify({ type: "doc", content: [] }),
      draftAttachments: [],
      draftRefs: {},
      origin: "history",
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
      loading: false,
      newChatWorkspace: null,
      interactions: [],
    };
    expect(sessionTabTitle(tab)).toBe("repo / 预览文本");
  });

  it("会话标签标题：无目录回退 workspace 目录名", () => {
    const tab: SessionTab = {
      id: "s1",
      kind: "chat",
      title: "",
      icon: "chat",
      threadId: null,
      name: "标题",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      taskMode: "execute",
      model: null,
      effort: null,
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
      loading: false,
      newChatWorkspace: null,
      interactions: [],
    };
    expect(sessionTabTitle(tab)).toBe("repo / 标题");
  });

  it("会话标签标题：全新标签兜底 目录名 / 新建会话", () => {
    const tab: SessionTab = {
      id: "s1",
      kind: "chat",
      title: "",
      icon: "chat",
      threadId: null,
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      taskMode: "execute",
      model: null,
      effort: null,
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
      loading: false,
      newChatWorkspace: null,
      interactions: [],
    };
    expect(sessionTabTitle(tab)).toBe("repo / 新建会话");
  });

  it("会话标签标题：新对话预选目录后为 目录名 / 新建会话", () => {
    const tab: SessionTab = {
      id: "s1",
      kind: "chat",
      title: "",
      icon: "chat",
      threadId: null,
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      taskMode: "execute",
      model: null,
      effort: null,
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
      loading: false,
      newChatWorkspace: "D:/projects/B",
      interactions: [],
    };
    expect(sessionTabTitle(tab)).toBe("B / 新建会话");
  });

  it("会话标签标题：无 workspace 且无 cwd 时降级为仅标题", () => {
    store.server.startupWorkspace = "";
    const tab: SessionTab = {
      id: "s1",
      kind: "chat",
      title: "",
      icon: "chat",
      threadId: null,
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      taskMode: "execute",
      model: null,
      effort: null,
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
      loading: false,
      newChatWorkspace: null,
      interactions: [],
    };
    expect(sessionTabTitle(tab)).toBe("新建会话");
  });

  it("thread/name/updated 更新会话标签名，不更新窗口标题", async () => {
    disposeEvents();
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockResolvedValue(undefined);
    __resetSessionTabsForTest();
    tabs.push({
      id: "s1",
      kind: "chat",
      title: "",
      icon: "chat",
      threadId: "t1",
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      taskMode: "execute",
      model: null,
      effort: null,
      draftJson: JSON.stringify({ type: "doc", content: [] }),
      draftAttachments: [],
      draftRefs: {},
      origin: "history",
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
      loading: false,
      newChatWorkspace: null,
      interactions: [],
    });
    activeTabId.value = "s1";
    await wireEvents();
    fireListen("thread/name/updated", {
      threadId: "t1",
      threadName: "新名",
    });
    expect(tabs[0].name).toBe("新名");
    expect(store.currentThreadName).toBe("新名");
    expect(mockWin.setTitle).not.toHaveBeenCalled();
    disposeEvents();
  });
});

describe("resolveSessionWorkspace 工作目录解析", () => {
  beforeEach(() => {
    store.server = {
      connected: false,
      startupWorkspace: "D:/repo",
      codexPath: null,
      logs: [],
    };
    store.currentThreadId = null;
    store.currentThreadWorkspace = null;
    store.newChatWorkspace = null;
  });

  it("有会话：取会话 cwd", () => {
    store.currentThreadId = "t1";
    store.currentThreadWorkspace = "D:/projects/other";
    store.newChatWorkspace = "D:/projects/B";
    expect(resolveSessionWorkspace()).toBe("D:/projects/other");
  });

  it("有会话但 cwd 缺失：回退 workspace，不读残留的 newChatWorkspace", () => {
    store.currentThreadId = "t1";
    store.currentThreadWorkspace = null;
    store.newChatWorkspace = "D:/projects/B";
    expect(resolveSessionWorkspace()).toBe("D:/repo");
  });

  it("无会话：优先 newChatWorkspace", () => {
    store.newChatWorkspace = "D:/projects/B";
    expect(resolveSessionWorkspace()).toBe("D:/projects/B");
  });

  it("全部为空时返回空串", () => {
    store.server.startupWorkspace = "";
    expect(resolveSessionWorkspace()).toBe("");
  });
});

describe("interaction:request 弹窗请求不抢窗口焦点", () => {
  beforeEach(() => {
    disposeEvents();
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    store.interactions = [];
    store.settings.sound_enabled = false;
    vi.mocked(getCurrentWindow).mockClear();
  });

  afterEach(() => {
    disposeEvents();
  });

  it("请求正确入队，且不调用任何窗口聚焦 API", async () => {
    await wireEvents();
    fireListen("interaction:request", {
      requestId: 99,
      method: "item/tool/requestUserInput",
      params: { questions: [{ id: "q1", question: "继续？", options: [] }] },
    });
    expect(store.interactions).toHaveLength(1);
    expect(store.interactions[0].requestId).toBe(99);
    expect(store.interactions[0].method).toBe("item/tool/requestUserInput");
    expect(getCurrentWindow).not.toHaveBeenCalled();
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

describe("多会话标签：新建/打开/切换/关闭", () => {
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
  });

  it("新建会话：创建新标签且不中断旧回合（会话多开）", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", { turnActive: true, currentTurnId: "turn-1" }),
    );
    activeTabId.value = "s1";
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    store.turnActive = true;

    expect(await newEmptyChat()).toBe(true);
    expect(store.confirm).toBeNull();
    expect(tabs).toHaveLength(2);
    expect(activeTabId.value).toBe(tabs[1].id);
    expect(store.currentThreadId).toBeNull();
    expect(store.currentTurnId).toBeNull();
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
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    store.turnActive = true;
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
    expect(store.currentThreadId).toBe("t2");
    expect(tabs[0].turnActive).toBe(true);
  });

  it("点击当前正在进行的会话（同线程）：不算切换，不中断、不重载", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", { turnActive: true, currentTurnId: "turn-1" }),
    );
    activeTabId.value = "s1";
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    store.turnActive = true;
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
    expect(store.turnActive).toBe(true);
    expect(store.currentTurnId).toBe("turn-1");
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
    store.currentThreadId = "t1";
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
    expect(store.currentThreadId).toBe("t2");
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
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    store.turnActive = true;

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
    expect(store.currentThreadId).toBeNull();
    expect(store.turnActive).toBe(false);
  });

  it("关闭运行中的会话标签：取消确认则保留、不中断", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", { turnActive: true, currentTurnId: "turn-1" }),
    );
    activeTabId.value = "s1";
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    store.turnActive = true;

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
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    store.turnActive = true;
    store.goalText = "目标";
    store.goalStatus = "active";
    store.goalArmed = true;

    await interrupt();
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", {
      threadId: "t1",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(store.goalText).toBeNull();
    expect(store.goalStatus).toBeNull();
    expect(store.goalArmed).toBe(false);
    expect(tabs[0].goalText).toBeNull();
  });

  it("显式传入旧线程/回合 id 时：目标清除作用于旧线程，且不污染新会话的回合 id", async () => {
    tabs.push(
      makeSessionTab("s2", "t2", {
        goalText: "旧目标",
        goalStatus: "active",
      }),
    );
    activeTabId.value = "s2";
    store.currentThreadId = "t2";
    store.currentTurnId = null;
    store.goalText = "旧目标";
    store.goalStatus = "active";
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
    expect(store.currentTurnId).toBeNull();
    expect(store.goalText).toBe("旧目标");
    expect(store.goalStatus).toBe("active");
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

  it("workspace 计算：有活动标签覆盖时返回覆盖值，否则回落会话工作区", async () => {
    store.currentThreadId = "t1";
    store.currentThreadWorkspace = "D:/session";
    store.server.startupWorkspace = "D:/startup";
    expect(workspace.value).toBe("D:/session");

    store.workspace = "D:/tab";
    expect(workspace.value).toBe("D:/tab");

    store.workspace = null;
    expect(workspace.value).toBe("D:/session");

    store.currentThreadId = null;
    store.newChatWorkspace = "D:/newchat";
    expect(workspace.value).toBe("D:/newchat");
  });

  it("会话标签 title 由同步点维护：新建带目录为 目录名 / 新建会话，改名后更新", async () => {
    store.server.startupWorkspace = "D:/repo";
    await newEmptyChat("D:/projects/B");
    expect(tabs[0].title).toBe("B / 新建会话");

    await newEmptyChat();
    const tab = tabs[1];
    expect(tab.title).toBe("repo / 新建会话");
    store.currentThreadId = "t1";
    store.currentThreadWorkspace = "D:/repo/sub";
    store.currentThreadName = "我的标题";
    await flushPromises();
    expect(tab.title).toBe("sub / 我的标题");
  });

  it("会话标签 loading 字段与 live 同步（loadingThread 改名后读写一致）", async () => {
    await newEmptyChat();
    const tab = tabs[0];
    expect(tab.loading).toBe(false);
    store.loading = true;
    await flushPromises();
    expect(tab.loading).toBe(true);
    store.loading = false;
    await flushPromises();
    expect(tab.loading).toBe(false);
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

  it("switchSessionTab：快照当前、恢复目标，各标签状态不串", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", {
        permissionMode: "full-access",
        taskMode: "plan",
        model: "gpt-5",
        effort: "high",
      }),
    );
    tabs.push(
      makeSessionTab("s2", "t2", {
        name: "会话B",
        workspace: "D:/repo/b",
        planPrompt: { threadId: "t2", turnId: "tp2", planText: "计划B" },
        permissionMode: "help-me-approve",
        taskMode: "execute",
        model: "o3",
        effort: "low",
      }),
    );
    activeTabId.value = "s1";
    store.currentThreadId = "t1";
    store.permissionMode = "full-access";
    store.taskMode = "plan";
    store.model = "gpt-5";
    store.effort = "high";
    store.turnActive = true;
    store.currentTurnId = "turn-1";
    store.goalText = "目标A";
    store.goalStatus = "active";
    store.attachments = [{ type: "text", text: "草稿A", text_elements: [] }];
    store.followupQueue = [{ text: "队列A", attachments: [] }];
    await flushPromises(); // 活动标签记录与 live 字段同步

    expect(await switchSessionTab("s2")).toBe(true);
    expect(activeTabId.value).toBe("s2");
    expect(store.currentThreadId).toBe("t2");
    expect(store.currentThreadName).toBe("会话B");
    expect(store.currentThreadWorkspace).toBe("D:/repo/b");
    expect(store.turnActive).toBe(false);
    expect(store.planPrompt?.planText).toBe("计划B");
    expect(store.permissionMode).toBe("help-me-approve");
    expect(store.taskMode).toBe("execute");
    expect(store.model).toBe("o3");
    expect(store.effort).toBe("low");

    expect(await switchSessionTab("s1")).toBe(true);
    expect(store.currentThreadId).toBe("t1");
    expect(store.turnActive).toBe(true);
    expect(store.currentTurnId).toBe("turn-1");
    expect(store.goalText).toBe("目标A");
    expect(store.attachments).toHaveLength(1);
    expect(store.followupQueue).toHaveLength(1);
    expect(store.permissionMode).toBe("full-access");
    expect(store.taskMode).toBe("plan");
    expect(store.model).toBe("gpt-5");
    expect(store.effort).toBe("high");
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

  it("活动标签的 live 字段变化自动同步回标签记录", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    store.currentThreadId = "t1";
    store.turnActive = true;
    await flushPromises();
    expect(tabs[0].turnActive).toBe(true);
    store.goalText = "新目标";
    await flushPromises();
    expect(tabs[0].goalText).toBe("新目标");
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

  it("closeAllSessionTabs：全部关闭后允许 0 标签", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    const skipped = await closeAllSessionTabs();
    expect(skipped).toBe(0);
    expect(tabs).toHaveLength(0);
    expect(activeTabId.value).toBe("");
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

  it("pickAndOpenNewSession：取消选择不新建", async () => {
    store.server.startupWorkspace = "D:/repo";
    mockedInvoke.mockResolvedValue(null);
    await pickAndOpenNewSession();
    expect(tabs).toHaveLength(0);
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

  it("interaction:request 按 threadId 路由到对应标签，resolved 移除", async () => {
    disposeEvents();
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    tabs.push(makeSessionTab("s1", "t1"));
    tabs.push(makeSessionTab("s2", "t2"));
    activeTabId.value = "s1";
    await wireEvents();
    fireListen("interaction:request", {
      requestId: 1,
      method: "item/commandExecution/requestApproval",
      params: { threadId: "t2", command: "npm test" },
    });
    expect(tabs[1].interactions).toHaveLength(1);
    expect(store.interactions).toHaveLength(0);

    fireListen("serverRequest/resolved", { requestId: 1, threadId: "t2" });
    expect(tabs[1].interactions).toHaveLength(0);
    disposeEvents();
  });

  it("后台标签 turn/completed：只更新该标签，不触碰活动标签", async () => {
    disposeEvents();
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    tabs.push(
      makeSessionTab("s1", "t1", { turnActive: true }),
    );
    tabs.push(
      makeSessionTab("s2", "t2", {
        turnActive: true,
        currentTurnId: "turn-2",
      }),
    );
    activeTabId.value = "s1";
    store.currentThreadId = "t1";
    store.turnActive = true;
    await wireEvents();
    fireListen("turn/completed", {
      threadId: "t2",
      turn: { id: "turn-2", status: "completed" },
    });
    expect(tabs[1].turnActive).toBe(false);
    expect(tabs[0].turnActive).toBe(true);
    expect(store.turnActive).toBe(true);
    disposeEvents();
  });

  it("后台标签 turn/completed：处理该标签的队列消息，不触碰活动标签", async () => {
    disposeEvents();
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      if (cmd === "turn_start") {
        return Promise.resolve({ turn: { id: "nt2" } });
      }
      return Promise.resolve(undefined);
    });
    tabs.push(
      makeSessionTab("s1", "t1", { turnActive: true }),
    );
    tabs.push(
      makeSessionTab("s2", "t2", {
        turnActive: true,
        currentTurnId: "turn-2",
        resumedThreadId: "t2",
        followupQueue: [{ text: "后台队列消息", attachments: [] }],
      }),
    );
    activeTabId.value = "s1";
    store.currentThreadId = "t1";
    store.turnActive = true;
    await wireEvents();
    fireListen("turn/completed", {
      threadId: "t2",
      turn: { id: "turn-2", status: "completed" },
    });
    await vi.waitFor(
      () => {
        expect(mockedInvoke).toHaveBeenCalledWith(
          "turn_start",
          expect.anything(),
        );
      },
      { timeout: 3000, interval: 20 },
    );
    expect(tabs[1].followupQueue).toHaveLength(0);
    expect(tabs[1].turnActive).toBe(true); // 新回合开始
    expect(tabs[1].currentTurnId).toBe("nt2");
    expect(tabs[0].turnActive).toBe(true);
    expect(store.turnActive).toBe(true);
    disposeEvents();
  });
});

describe("GoalStatus 枚举对齐协议 ThreadGoalStatus", () => {
  it("接受协议全部取值（active/paused/blocked/usageLimited/budgetLimited/complete）", () => {
    expect(isGoalStatus("active")).toBe(true);
    expect(isGoalStatus("paused")).toBe(true);
    expect(isGoalStatus("blocked")).toBe(true);
    expect(isGoalStatus("usageLimited")).toBe(true);
    expect(isGoalStatus("budgetLimited")).toBe(true);
    expect(isGoalStatus("complete")).toBe(true);
  });

  it("拒绝旧版/非协议取值（completed/budget_limited/cleared 等）", () => {
    expect(isGoalStatus("completed")).toBe(false);
    expect(isGoalStatus("budget_limited")).toBe(false);
    expect(isGoalStatus("cleared")).toBe(false);
    expect(isGoalStatus("")).toBe(false);
    expect(isGoalStatus(null)).toBe(false);
    expect(isGoalStatus(undefined)).toBe(false);
  });
});

describe("线程级目标：设置/清除/读取/事件同步", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.currentThreadId = null;
    store.currentThreadName = "";
    store.currentThreadWorkspace = null;
    store.currentThreadOrigin = null;
    store.resumedThreadId = null;
    store.turnActive = false;
    store.turnInterrupted = false;
    store.currentTurnId = null;
    store.threadTokenUsage = null;
    store.goalText = null;
    store.goalStatus = null;
    store.toast = "";
  });

  it("setGoal：无会话时不挂载（返回 false，不调用 goal_set）", async () => {
    store.currentThreadId = null;
    const ok = await setGoal("修复登录");
    expect(ok).toBe(false);
    expect(store.goalText).toBeNull();
    expect(store.goalStatus).toBeNull();
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "goal_set",
      expect.anything(),
    );
  });

  it("setGoal：空文本不提交", async () => {
    store.currentThreadId = "t1";
    const ok = await setGoal("   ");
    expect(ok).toBe(false);
    expect(store.toast).toContain("目标不能为空");
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "goal_set",
      expect.anything(),
    );
  });

  it("setGoal：超过 4000 字符被拒绝", async () => {
    store.currentThreadId = "t1";
    const ok = await setGoal("长".repeat(4001));
    expect(ok).toBe(false);
    expect(store.toast).toContain("4000");
  });

  it("setGoal：成功后本地记录目标与 active 状态", async () => {
    store.currentThreadId = "t1";
    mockedInvoke.mockResolvedValue(undefined);
    const ok = await setGoal("  修复登录流程  ");
    expect(ok).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("goal_set", {
      threadId: "t1",
      objective: "修复登录流程",
    });
    expect(store.goalText).toBe("修复登录流程");
    expect(store.goalStatus).toBe("active");
    expect(store.toast).toContain("已设置目标");
  });

  it("setGoal：失败时保留原目标并提示错误", async () => {
    store.currentThreadId = "t1";
    store.goalText = "旧目标";
    store.goalStatus = "complete";
    mockedInvoke.mockRejectedValue(new Error("服务端拒绝"));
    const ok = await setGoal("新目标");
    expect(ok).toBe(false);
    expect(store.goalText).toBe("旧目标");
    expect(store.goalStatus).toBe("complete");
    expect(store.toast).toContain("服务端拒绝");
  });

  it("clearGoal：无会话时仅清空本地状态", async () => {
    store.goalText = "目标";
    store.goalStatus = "active";
    store.goalArmed = true;
    await clearGoal();
    expect(store.goalText).toBeNull();
    expect(store.goalStatus).toBeNull();
    expect(store.goalArmed).toBe(false);
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "goal_clear",
      expect.anything(),
    );
  });

  it("空闲但线程有活跃目标时新建会话：不清目标（会话多开，后台目标继续）", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", {
        goalText: "旧目标",
        goalStatus: "active",
        goalArmed: true,
      }),
    );
    activeTabId.value = "s1";
    store.currentThreadId = "t1";
    store.goalText = "旧目标";
    store.goalStatus = "active";
    store.goalArmed = true;
    expect(await newEmptyChat()).toBe(true);
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "goal_clear",
      expect.anything(),
    );
    expect(tabs[0].goalText).toBe("旧目标");
    expect(tabs[0].goalStatus).toBe("active");
  });

  it("空闲但旧线程有活跃目标时切换历史会话：不清目标", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", {
        goalText: "旧目标",
        goalStatus: "active",
      }),
    );
    activeTabId.value = "s1";
    store.currentThreadId = "t1";
    store.goalText = "旧目标";
    store.goalStatus = "active";
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
    const p = openThread("t2");
    expect(await p).toBe(true);
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "goal_clear",
      expect.anything(),
    );
    expect(store.currentThreadId).toBe("t2");
    expect(tabs[0].goalText).toBe("旧目标");
  });

  it("openThread：goal_get 返回终态时 toast + 复位 + goal_clear", async () => {
    store.currentThreadId = "t1";
    store.goalArmed = true;
    store.toast = "";
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
      if (cmd === "goal_get") {
        return Promise.resolve({ objective: "修复登录", status: "complete" });
      }
      return Promise.resolve(undefined);
    });
    const p = openThread("t2");
    expect(await p).toBe(true);
    expect(store.currentThreadId).toBe("t2");
    // 终态目标：打开即 toast 提示并复位（相当于没有目标），服务端同步清除
    expect(store.goalText).toBeNull();
    expect(store.goalStatus).toBeNull();
    expect(store.goalArmed).toBe(false);
    expect(store.toast).toContain("目标已完成");
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", { threadId: "t2" });
  });

  it("openThread：goal_get 兼容 {goal:{objective,status}} 包裹返回", async () => {
    store.currentThreadId = "t1";
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
      if (cmd === "goal_get") {
        return Promise.resolve({
          goal: { objective: "重构登录", status: "active" },
        });
      }
      return Promise.resolve(undefined);
    });
    const p = openThread("t2");
    expect(await p).toBe(true);
    expect(store.goalText).toBe("重构登录");
    expect(store.goalStatus).toBe("active");
  });

  it("openThread：goal_get 失败或未挂目标时状态为空", async () => {
    store.currentThreadId = "t1";
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
      if (cmd === "goal_get") return Promise.reject(new Error("读取失败"));
      return Promise.resolve(undefined);
    });
    const p = openThread("t2");
    expect(await p).toBe(true);
    expect(store.goalText).toBeNull();
    expect(store.goalStatus).toBeNull();
  });

  it("openThread：loadFullItems 以 asc+full 拉取完整工具/命令详情", async () => {
    store.currentThreadId = "t1";
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
    const p = openThread("t2");
    expect(await p).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/turns/list",
      params: {
        threadId: "t2",
        cursor: null,
        limit: 50,
        // 协议 SortDirection 为 "asc" | "desc"（旧值 "ascending" 会被服务端拒绝）
        sortDirection: "asc",
        itemsView: "full",
      },
    });
  });

  it("continueTurn：待挂载目标在回合启动前 goal_set 挂载", async () => {
    store.currentThreadId = "t1";
    store.resumedThreadId = "t1"; // 跳过 thread_resume
    store.goalText = "修复登录";
    store.goalStatus = null;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "turn_start") {
        return Promise.resolve({ turn: { id: "nt1" } });
      }
      return Promise.resolve(undefined);
    });
    await sendPrompt("你好");
    expect(mockedInvoke).toHaveBeenCalledWith("goal_set", {
      threadId: "t1",
      objective: "修复登录",
    });
    expect(store.goalStatus).toBe("active");
    const goalIdx = mockedInvoke.mock.calls.findIndex(
      ([c]) => c === "goal_set",
    );
    const turnIdx = mockedInvoke.mock.calls.findIndex(
      ([c]) => c === "turn_start",
    );
    expect(goalIdx).toBeGreaterThan(-1);
    expect(turnIdx).toBeGreaterThan(goalIdx);
  });
});

describe("thread/goal 事件同步与回合完成不清目标", () => {
  beforeEach(() => {
    disposeEvents(); // 重置 wired，确保本组用例重新注册监听
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    store.currentThreadId = "t1";
    store.turnActive = false;
    store.turnInterrupted = false;
    store.currentTurnId = null;
    store.taskMode = "execute";
    store.goalText = null;
    store.goalStatus = null;
  });

  it("thread/goal/updated：同步当前会话的目标文本与状态", async () => {
    await wireEvents();
    fireListen("thread/goal/updated", {
      threadId: "t1",
      goal: { objective: "发布 v2", status: "active" },
    });
    expect(store.goalText).toBe("发布 v2");
    expect(store.goalStatus).toBe("active");
  });

  it("thread/goal/updated 终态：自动清目标并复位 flag（goal_clear）", async () => {
    await wireEvents();
    store.goalText = "发布 v2";
    store.goalStatus = "active";
    store.toast = "";
    fireListen("thread/goal/updated", {
      threadId: "t1",
      goal: { objective: "发布 v2", status: "complete" },
    });
    await vi.waitFor(() => expect(store.goalText).toBeNull(), {
      timeout: 3000,
      interval: 20,
    });
    expect(store.goalStatus).toBeNull();
    expect(store.goalArmed).toBe(false);
    expect(store.toast).toContain("目标已完成");
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", { threadId: "t1" });
  });

  it("thread/goal/updated：旧会话通知不污染当前会话", async () => {
    await wireEvents();
    store.goalText = "当前目标";
    store.goalStatus = "active";
    fireListen("thread/goal/updated", {
      threadId: "t-other",
      goal: { objective: "别人", status: "complete" },
    });
    expect(store.goalText).toBe("当前目标");
    expect(store.goalStatus).toBe("active");
  });

  it("thread/goal/cleared：清空当前会话目标", async () => {
    await wireEvents();
    store.goalText = "发布 v2";
    store.goalStatus = "active";
    store.goalArmed = true;
    fireListen("thread/goal/cleared", { threadId: "t1" });
    expect(store.goalText).toBeNull();
    expect(store.goalStatus).toBeNull();
    expect(store.goalArmed).toBe(false);
  });

  it("turn/completed：不再自动清除目标（目标由服务端循环管理）", async () => {
    await wireEvents();
    store.goalText = "发布 v2";
    store.goalStatus = "active";
    store.itemsByThread["t1"] = [];
    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-1", status: "completed" },
    });
    await vi.waitFor(() => expect(store.turnActive).toBe(false), {
      timeout: 3000,
      interval: 20,
    });
    expect(store.goalText).toBe("发布 v2");
    expect(store.goalStatus).toBe("active");
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

describe("置顶 togglePin（新版 Pinned 分区协议）", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetPinnedSectionForTest();
    store.threads = [];
    store.loadingHistory = false;
    store.toast = "";
  });

  function rpcCalls(method: string) {
    return mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string })?.method === method,
    );
  }

  function capabilityCalls() {
    return mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "codex_pin_capability",
    );
  }

  function mockCapability(
    protocol: string,
    pinnedSectionId: string | null,
    refreshedPinned = true,
  ) {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_pin_capability") {
        return Promise.resolve({ protocol, pinnedSectionId });
      }
      if (cmd === "codex_rpc") {
        return Promise.resolve({ thread: { id: "t1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({
          data: [
            {
              id: "t1",
              name: "会话",
              createdAt: 0,
              recencyAt: 0,
              isPinned: refreshedPinned,
            },
          ],
          nextCursor: null,
        });
      }
      return Promise.resolve(undefined);
    });
  }

  it("metadata_section 置顶：探测到分区协议后用 Pinned 分区 id 调用 metadata/update", async () => {
    mockCapability("metadata_section", "sec-1", true);
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(rpcCalls("thread/metadata/update")).toHaveLength(1);
    expect(rpcCalls("thread/metadata/update")[0][1]).toEqual({
      method: "thread/metadata/update",
      params: { threadId: "t1", sectionId: "sec-1" },
    });
    expect(store.threads[0].isPinned).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("thread_list", {
      limit: 50,
      cursor: null,
    });
  });

  it("metadata_section 取消置顶：sectionId 传 null", async () => {
    mockCapability("metadata_section", "sec-1", false);
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", false);

    expect(rpcCalls("thread/metadata/update")).toHaveLength(1);
    expect(rpcCalls("thread/metadata/update")[0][1]).toEqual({
      method: "thread/metadata/update",
      params: { threadId: "t1", sectionId: null },
    });
    expect(store.threads[0].isPinned).toBe(false);
  });

  it("section_move 置顶/取消：走新版 threadSection/move 协议", async () => {
    let refreshedPinned = true;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_pin_capability") {
        return Promise.resolve({ protocol: "section_move", pinnedSectionId: "sec-1" });
      }
      if (cmd === "codex_rpc") {
        return Promise.resolve({ thread: { id: "t1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({
          data: [
            {
              id: "t1",
              name: "会话",
              createdAt: 0,
              recencyAt: 0,
              isPinned: refreshedPinned,
            },
          ],
          nextCursor: null,
        });
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(rpcCalls("threadSection/move")).toHaveLength(1);
    expect(rpcCalls("threadSection/move")[0][1]).toEqual({
      method: "threadSection/move",
      params: { threadId: "t1", sectionId: "sec-1" },
    });
    expect(store.threads[0].isPinned).toBe(true);

    refreshedPinned = false;
    await togglePin("t1", false);

    expect(rpcCalls("threadSection/move")).toHaveLength(2);
    expect(rpcCalls("threadSection/move")[1][1]).toEqual({
      method: "threadSection/move",
      params: { threadId: "t1", sectionId: null },
    });
    expect(store.threads[0].isPinned).toBe(false);
  });

  it("section_move 新方法名：探测返回 thread/section/move 时按其调用", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_pin_capability") {
        return Promise.resolve({
          protocol: "section_move",
          pinnedSectionId: "sec-1",
          sectionMoveMethod: "thread/section/move",
        });
      }
      if (cmd === "codex_rpc") {
        return Promise.resolve({ thread: { id: "t1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({
          data: [
            {
              id: "t1",
              name: "会话",
              createdAt: 0,
              recencyAt: 0,
              isPinned: true,
            },
          ],
          nextCursor: null,
        });
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(rpcCalls("thread/section/move")).toHaveLength(1);
    expect(rpcCalls("thread/section/move")[0][1]).toEqual({
      method: "thread/section/move",
      params: { threadId: "t1", sectionId: "sec-1" },
    });
    expect(rpcCalls("threadSection/move")).toHaveLength(0);
    expect(store.threads[0].isPinned).toBe(true);
  });

  it("metadata_is_pinned 置顶/取消：旧版走 metadata/update isPinned 布尔", async () => {
    let refreshedPinned = true;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_pin_capability") {
        return Promise.resolve({ protocol: "metadata_is_pinned", pinnedSectionId: null });
      }
      if (cmd === "codex_rpc") {
        return Promise.resolve({ thread: { id: "t1" } });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({
          data: [
            {
              id: "t1",
              name: "会话",
              createdAt: 0,
              recencyAt: 0,
              isPinned: refreshedPinned,
            },
          ],
          nextCursor: null,
        });
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(rpcCalls("thread/metadata/update")).toHaveLength(1);
    expect(rpcCalls("thread/metadata/update")[0][1]).toEqual({
      method: "thread/metadata/update",
      params: { threadId: "t1", isPinned: true },
    });
    expect(store.threads[0].isPinned).toBe(true);

    refreshedPinned = false;
    await togglePin("t1", false);

    expect(rpcCalls("thread/metadata/update")).toHaveLength(2);
    expect(rpcCalls("thread/metadata/update")[1][1]).toEqual({
      method: "thread/metadata/update",
      params: { threadId: "t1", isPinned: false },
    });
    expect(store.threads[0].isPinned).toBe(false);
  });

  it("unsupported：只弹提示不发请求，本地状态不变", async () => {
    mockCapability("unsupported", null);
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(rpcCalls("thread/metadata/update")).toHaveLength(0);
    expect(rpcCalls("threadSection/move")).toHaveLength(0);
    expect(store.threads[0].isPinned).toBeUndefined();
    expect(store.toast).toContain("不支持置顶");
  });

  it("探测失败：提示且不发起置顶请求，便于下次重试", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_pin_capability") {
        return Promise.reject(new Error("server not ready"));
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(rpcCalls("thread/metadata/update")).toHaveLength(0);
    expect(rpcCalls("threadSection/move")).toHaveLength(0);
    expect(store.threads[0].isPinned).toBeUndefined();
    expect(store.toast).toContain("不支持置顶");
  });

  it("探测结果缓存：连续两次置顶只探测一次", async () => {
    mockCapability("metadata_is_pinned", null);
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);
    await togglePin("t1", false);

    expect(capabilityCalls()).toHaveLength(1);
  });

  it("写失败时回滚本地置顶状态并提示", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "codex_pin_capability") {
        return Promise.resolve({ protocol: "metadata_section", pinnedSectionId: "sec-1" });
      }
      if (cmd === "codex_rpc" && (args as { method?: string })?.method === "thread/metadata/update") {
        return Promise.reject(new Error("update failed"));
      }
      return Promise.resolve(undefined);
    });
    store.threads = [{ id: "t1", name: "会话", createdAt: 0, recencyAt: 0 }];

    await togglePin("t1", true);

    expect(store.threads[0].isPinned).toBeUndefined();
    expect(rpcCalls("thread/metadata/update")).toHaveLength(1);
  });

  it("sortThreads 从 section 推导置顶：固定优先，再按最近时间降序", () => {
    const list = [
      { id: "b", name: "未置顶新", createdAt: 0, recencyAt: 2 },
      { id: "a", name: "置顶旧", createdAt: 0, recencyAt: 1, section: { id: "sec-1", name: "Pinned" } },
      { id: "c", name: "未置顶更新", createdAt: 0, recencyAt: 3 },
    ];
    const sorted = sortThreads(list);
    expect(sorted.map((t) => t.id)).toEqual(["a", "c", "b"]);
    expect(sorted[0].isPinned).toBe(true);
  });
});

describe("sanitizeTitle 标题清洗", () => {
  it("去掉引号与 Markdown 标记、折叠空白", () => {
    expect(sanitizeTitle('  "修复 **登录** 页报错" ')).toBe("修复 登录 页报错");
    expect(sanitizeTitle("`重构` 模块\n\n换行\t空白")).toBe("重构 模块 换行 空白");
    expect(sanitizeTitle("标题。")).toBe("标题。");
  });

  it("空输入返回空串，超长截断 50 字", () => {
    expect(sanitizeTitle("   \n ")).toBe("");
    expect(sanitizeTitle("很".repeat(60))).toHaveLength(50);
  });
});

describe("autoTitleThread 临时线程标题总结", () => {
  const LONG_TEXT = "这是一个非常长的用户消息，用来验证标题总结功能能否正常触发和写回。".repeat(2);

  beforeEach(() => {
    disposeEvents(); // 重置 wired，避免前面 wireEvents 用例的监听残留
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    __resetTitleHelperCapabilityForTest();
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    store.toast = "";
    store.currentThreadId = "t1";
    store.currentThreadName = "";
    store.currentThreadWorkspace = "D:/repo";
    store.server.startupWorkspace = "D:/repo";
    store.threads = [{ id: "t1", name: null, preview: "旧预览", createdAt: 0, recencyAt: 0 }];
  });

  it("不支持 experimentalApi：完全不发起临时线程", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: false, ephemeral: false });
      }
      return Promise.resolve(undefined);
    });
    await autoTitleThread("t1", LONG_TEXT);
    expect(mockedInvoke).not.toHaveBeenCalledWith("thread_start", expect.anything());
  });

  it("短文保持默认标题，不消耗模型", async () => {
    await autoTitleThread("t1", "短消息");
    expect(mockedInvoke).not.toHaveBeenCalledWith("thread_start", expect.anything());
  });

  it("阈值边界：纯文本 15 字不触发，16 字触发", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: true, ephemeral: true });
      }
      if (cmd === "thread_start") return Promise.resolve({ thread: { id: "helper1" } });
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "ht1" } });
      return Promise.resolve(undefined);
    });

    await autoTitleThread("t1", "一二三四五六七八九十一二三四五"); // 15 字
    expect(mockedInvoke).not.toHaveBeenCalledWith("thread_start", expect.anything());

    mockedInvoke.mockClear();
    const p = autoTitleThread("t1", "一二三四五六七八九十一二三四五六"); // 16 字
    await p;
    expect(mockedInvoke).toHaveBeenCalledWith("thread_start", expect.anything());

    // 结算临时回合并清理，避免 30s 兜底定时器悬空
    fireListen("turn/completed", {
      threadId: "helper1",
      turn: { id: "ht1", status: "interrupted" },
    });
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
        method: "thread/unsubscribe",
        params: { threadId: "helper1" },
      });
    }, { timeout: 3000, interval: 20 });
  });

  it("线程已有名称时不覆盖", async () => {
    const tab = tabs[0];
    tab.name = "手动标题";
    tab.nameIsFirstMessage = false;
    store.currentThreadName = "手动标题";
    await autoTitleThread("t1", LONG_TEXT);
    expect(mockedInvoke).not.toHaveBeenCalledWith("thread_start", expect.anything());
  });

  it("名称来自首条消息时总结可覆盖并复位标记", async () => {
    const tab = tabs[0];
    tab.name = "帮我修复登录页面报错";
    tab.nameIsFirstMessage = true;
    store.currentThreadName = "帮我修复登录页面报错";
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: true, ephemeral: true });
      }
      if (cmd === "thread_start") return Promise.resolve({ thread: { id: "helper1" } });
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "ht1" } });
      if (cmd === "thread_set_name") return Promise.resolve({});
      if (cmd === "thread_list")
        return Promise.resolve({ data: [], nextCursor: null });
      return Promise.resolve(undefined);
    });

    const p = autoTitleThread("t1", LONG_TEXT);
    await p;
    fireListen("item/agentMessage/delta", {
      threadId: "helper1",
      itemId: "m1",
      delta: "修复登录页面报错",
    });
    fireListen("turn/completed", {
      threadId: "helper1",
      turn: { id: "ht1", status: "completed" },
    });

    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("thread_set_name", {
        threadId: "t1",
        name: "修复登录页面报错",
      });
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(tab.nameIsFirstMessage).toBe(false);
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("thread_list", expect.anything());
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
        method: "thread/unsubscribe",
        params: { threadId: "helper1" },
      });
    }, { timeout: 3000, interval: 20 });
  });

  it("ephemeral 路径：模型标题写回，临时线程注销", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: true, ephemeral: true });
      }
      if (cmd === "thread_start") {
        const params = (args as { params?: Record<string, unknown> }).params ?? {};
        expect(params.ephemeral).toBe(true);
        expect(params.model).toBeUndefined();
        expect(params.sandbox).toBe("read-only");
        expect(params.cwd).toBe("D:/repo");
        return Promise.resolve({ thread: { id: "helper1" } });
      }
      if (cmd === "turn_start") {
        const params = (args as { params?: Record<string, unknown> }).params ?? {};
        expect(
          (params.sandboxPolicy as { type?: string } | undefined)?.type,
        ).toBe("readOnly");
        return Promise.resolve({ turn: { id: "ht1" } });
      }
      if (cmd === "thread_set_name") return Promise.resolve({});
      if (cmd === "thread_list")
        return Promise.resolve({ data: [], nextCursor: null });
      return Promise.resolve(undefined);
    });

    const p = autoTitleThread("t1", LONG_TEXT);
    await p;
    fireListen("item/agentMessage/delta", {
      threadId: "helper1",
      itemId: "m1",
      delta: "修复登录",
    });
    fireListen("item/agentMessage/delta", {
      threadId: "helper1",
      itemId: "m1",
      delta: "页面报错问题",
    });
    fireListen("turn/completed", {
      threadId: "helper1",
      turn: { id: "ht1", status: "completed" },
    });

    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("thread_set_name", {
        threadId: "t1",
        name: "修复登录页面报错问题",
      });
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(store.toast).toContain("当前会话的标题已简化");
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
        method: "thread/unsubscribe",
        params: { threadId: "helper1" },
      });
    }, { timeout: 3000, interval: 20 });
  });

  it("不支持 ephemeral：普通线程总结后删除", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: true, ephemeral: false });
      }
      if (cmd === "thread_start") {
        const params = (args as { params?: Record<string, unknown> }).params ?? {};
        expect(params.ephemeral).toBeUndefined();
        expect(params.model).toBeUndefined();
        expect(params.sandbox).toBe("read-only");
        return Promise.resolve({ thread: { id: "helper1" } });
      }
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "ht1" } });
      if (cmd === "thread_set_name") return Promise.resolve({});
      if (cmd === "thread_list")
        return Promise.resolve({ data: [], nextCursor: null });
      return Promise.resolve(undefined);
    });

    const p = autoTitleThread("t1", LONG_TEXT);
    await p;
    fireListen("item/agentMessage/delta", {
      threadId: "helper1",
      itemId: "m1",
      delta: "重构模块",
    });
    fireListen("turn/completed", {
      threadId: "helper1",
      turn: { id: "ht1", status: "completed" },
    });

    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("thread_set_name", {
        threadId: "t1",
        name: "重构模块",
      });
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(store.toast).toContain("当前会话的标题已简化");
    }, { timeout: 3000, interval: 20 });
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("thread_delete", {
        threadId: "helper1",
      });
    }, { timeout: 3000, interval: 20 });
  });

  it("回合失败：不写回标题，仍清理临时线程", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: true, ephemeral: true });
      }
      if (cmd === "thread_start") return Promise.resolve({ thread: { id: "helper1" } });
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "ht1" } });
      return Promise.resolve(undefined);
    });

    const p = autoTitleThread("t1", LONG_TEXT);
    await p;
    fireListen("turn/completed", {
      threadId: "helper1",
      turn: { id: "ht1", status: "failed" },
    });

    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
        method: "thread/unsubscribe",
        params: { threadId: "helper1" },
      });
    }, { timeout: 3000, interval: 20 });
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "thread_set_name",
      expect.anything(),
    );
    expect(store.toast).not.toContain("当前会话的标题已简化");
  });

  it("后台临时线程事件被隔离：不影响全局进行中状态", async () => {
    store.turnActive = false;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: true, ephemeral: true });
      }
      if (cmd === "thread_start") return Promise.resolve({ thread: { id: "helper1" } });
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "ht1" } });
      if (cmd === "thread_set_name") return Promise.resolve({});
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });

    await wireEvents();
    const p = autoTitleThread("t1", LONG_TEXT);
    await p;

    // 后台线程 turn/started：不置为进行中
    fireListen("turn/started", { threadId: "helper1", turn: { id: "ht1" } });
    expect(store.turnActive).toBe(false);

    // 后台线程 turn/completed：不结束全局状态、不触发历史刷新
    fireListen("item/agentMessage/delta", {
      threadId: "helper1",
      itemId: "m1",
      delta: "后台标题",
    });
    fireListen("turn/completed", {
      threadId: "helper1",
      turn: { id: "ht1", status: "completed" },
    });
    await vi.waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("thread_set_name", {
        threadId: "t1",
        name: expect.any(String),
      });
    }, { timeout: 3000, interval: 20 });
    expect(store.turnActive).toBe(false);

    // 主线程事件照常工作
    fireListen("turn/started", { threadId: "t1", turn: { id: "mt1" } });
    expect(store.turnActive).toBe(true);
    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "mt1", status: "completed" },
    });
    await vi.waitFor(() => expect(store.turnActive).toBe(false), {
      timeout: 3000,
      interval: 20,
    });
  });
});

describe("消息变更计数器与回合结束清扫", () => {
  beforeEach(() => {
    disposeEvents(); // 重置 wired，确保本组用例重新注册监听
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    store.itemsRev = 0;
    store.currentThreadId = "t1";
    store.turnActive = false;
    store.turnInterrupted = false;
    store.currentTurnId = null;
    store.activeWorkByThread = {};
  });

  it("流式增量事件（文本/命令输出/思考过程）递增 itemsRev", async () => {
    await wireEvents();
    store.itemsByThread["t1"] = [
      { id: "a1", type: "agentMessage", text: "", streaming: true },
      {
        id: "c1",
        type: "commandExecution",
        command: "",
        status: "in_progress",
        aggregatedOutput: "",
      },
      { id: "r1", type: "reasoning", content: [] },
    ];

    fireListen("item/agentMessage/delta", {
      threadId: "t1",
      itemId: "a1",
      delta: "你好",
    });
    expect(store.itemsRev).toBe(1);

    fireListen("item/commandExecution/outputDelta", {
      threadId: "t1",
      itemId: "c1",
      delta: "out",
    });
    expect(store.itemsRev).toBe(2);

    fireListen("item/reasoning/textDelta", {
      threadId: "t1",
      itemId: "r1",
      delta: "思考",
      contentIndex: 0,
    });
    expect(store.itemsRev).toBe(3);
  });

  it("回合结束清扫：进行中/流式 item 置为 interrupted 并补算耗时", async () => {
    await wireEvents();
    store.itemsByThread["t1"] = [
      {
        id: "c1",
        type: "commandExecution",
        command: "npm run build",
        status: "in_progress",
        streaming: true,
        startedAtMs: 1000,
      },
      {
        id: "m1",
        type: "agentMessage",
        text: "已完成",
        status: "completed",
        durationMs: 5,
      },
    ];
    store.activeWorkByThread["t1"] = 1;
    const before = store.itemsRev;

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-1", status: "interrupted" },
    });
    await vi.waitFor(() => expect(store.turnActive).toBe(false), {
      timeout: 3000,
      interval: 20,
    });

    const c1 = store.itemsByThread["t1"][0];
    expect(c1.status).toBe("interrupted");
    expect(c1.streaming).toBe(false);
    expect(typeof c1.durationMs).toBe("number");
    expect(c1.durationMs).toBeGreaterThanOrEqual(0);
    // 已完成的 item 不受影响
    expect(store.itemsByThread["t1"][1].status).toBe("completed");
    // 进行中计数同步归零，变更计数递增
    expect(store.activeWorkByThread["t1"]).toBe(0);
    expect(store.itemsRev).toBe(before + 1);
  });

  it("非中断完成时对残留进行中项兜底标为 canceled", async () => {
    await wireEvents();
    store.itemsByThread["t1"] = [
      {
        id: "c1",
        type: "commandExecution",
        command: "x",
        status: "in_progress",
        streaming: true,
        startedAtMs: 1000,
      },
    ];
    store.activeWorkByThread["t1"] = 1;

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-1", status: "completed" },
    });
    await vi.waitFor(() => expect(store.turnActive).toBe(false), {
      timeout: 3000,
      interval: 20,
    });

    expect(store.itemsByThread["t1"][0].status).toBe("canceled");
    expect(store.activeWorkByThread["t1"]).toBe(0);
  });
});

describe("旧会话 turn 事件不串扰新会话", () => {
  beforeEach(() => {
    disposeEvents(); // 重置 wired，确保本组用例重新注册监听
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    store.currentThreadId = "t2";
    store.resumedThreadId = "t2";
    store.taskMode = "execute";
    store.turnActive = false;
    store.turnInterrupted = false;
    store.currentTurnId = null;
    store.planPrompt = null;
    store.followupQueue = [];
    store.itemsByThread = {};
    store.activeWorkByThread = {};
  });

  afterEach(() => {
    disposeEvents();
  });

  it("旧线程 turn/completed：不复位状态、不弹计划确认、不消费队列", async () => {
    await wireEvents();
    store.turnActive = true; // 模拟新会话正在运行
    store.taskMode = "plan";
    store.followupQueue.push({ text: "队列消息", attachments: [] });
    store.itemsByThread["t1"] = [
      { id: "p1", type: "plan", text: "旧会话计划", status: "completed" },
    ];

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "old-turn", status: "completed" },
    });
    await flushPromises();

    expect(store.turnActive).toBe(true);
    expect(store.currentTurnId).toBeNull();
    expect(store.planPrompt).toBeNull();
    expect(store.followupQueue).toHaveLength(1);
  });

  it("旧线程 turn/started：不把新会话置为进行中", async () => {
    await wireEvents();
    fireListen("turn/started", { threadId: "t1", turn: { id: "old-turn" } });
    expect(store.turnActive).toBe(false);
    expect(store.currentTurnId).toBeNull();
  });
});

describe("后台临时线程 delta 事件隔离", () => {
  const LONG_TEXT = "这是一个非常长的用户消息，用来验证标题总结功能能否正常触发和写回。".repeat(2);

  beforeEach(() => {
    disposeEvents();
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    __resetTitleHelperCapabilityForTest();
    store.toast = "";
    store.currentThreadId = "t1";
    store.currentThreadName = "";
    store.currentThreadWorkspace = "D:/repo";
    store.server.startupWorkspace = "D:/repo";
    store.threads = [
      { id: "t1", name: null, preview: "旧预览", createdAt: 0, recencyAt: 0 },
    ];
    store.itemsByThread = {};
    store.activeWorkByThread = {};
    store.itemsRev = 0;
  });

  afterEach(() => {
    disposeEvents();
  });

  it("命令输出/思考/文件变更 delta 不进入全局状态", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_title_helper_capability") {
        return Promise.resolve({ experimentalApi: true, ephemeral: true });
      }
      if (cmd === "thread_start") return Promise.resolve({ thread: { id: "helper1" } });
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "ht1" } });
      return Promise.resolve(undefined);
    });

    await wireEvents();
    const p = autoTitleThread("t1", LONG_TEXT);
    await p;
    expect(store.itemsRev).toBe(0);

    fireListen("item/commandExecution/outputDelta", {
      threadId: "helper1",
      itemId: "c1",
      delta: "out",
    });
    fireListen("item/reasoning/textDelta", {
      threadId: "helper1",
      itemId: "r1",
      delta: "思考",
      contentIndex: 0,
    });
    fireListen("item/fileChange/patchUpdated", {
      threadId: "helper1",
      itemId: "f1",
      changes: [{ path: "a.txt", kind: "add", diff: "+x" }],
    });
    await flushPromises();

    expect(store.itemsByThread["helper1"]).toBeUndefined();
    expect(store.activeWorkByThread["helper1"]).toBeUndefined();
    expect(store.itemsRev).toBe(0);

    // 结算临时回合并清理，避免 30s 兜底定时器悬空
    fireListen("turn/completed", {
      threadId: "helper1",
      turn: { id: "ht1", status: "interrupted" },
    });
    await flushPromises();
  });
});

describe("计划完成确认弹窗", () => {
  beforeEach(() => {
    disposeEvents(); // 重置 wired，确保本组用例重新注册监听
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    store.currentThreadId = "t1";
    store.resumedThreadId = "t1";
    store.taskMode = "plan";
    store.turnActive = false;
    store.turnInterrupted = false;
    store.currentTurnId = null;
    store.followupQueue = [];
    store.planPrompt = null;
    store.itemsByThread = {};
  });

  it("计划模式回合正常完成且含 plan 内容 → 弹出计划确认", async () => {
    await wireEvents();
    store.itemsByThread["t1"] = [
      { id: "p1", type: "plan", text: "# 修复方案\n1. 改代码", status: "completed" },
    ];

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-1", status: "completed" },
    });
    await vi.waitFor(() => expect(store.planPrompt).not.toBeNull(), {
      timeout: 3000,
      interval: 20,
    });

    expect(store.planPrompt).toEqual({
      threadId: "t1",
      turnId: "turn-1",
      planText: "# 修复方案\n1. 改代码",
    });
  });

  it("取最后一条 plan item 的文本", async () => {
    await wireEvents();
    store.itemsByThread["t1"] = [
      { id: "p1", type: "plan", text: "旧计划", status: "completed" },
      { id: "p2", type: "plan", text: "新计划", status: "completed" },
    ];

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-1", status: "completed" },
    });
    await vi.waitFor(() => expect(store.planPrompt?.planText).toBe("新计划"), {
      timeout: 3000,
      interval: 20,
    });
  });

  it("中断回合不弹窗", async () => {
    await wireEvents();
    store.itemsByThread["t1"] = [
      { id: "p1", type: "plan", text: "# 计划", status: "completed" },
    ];

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-1", status: "interrupted" },
    });
    await vi.waitFor(() => expect(store.turnActive).toBe(false), {
      timeout: 3000,
      interval: 20,
    });
    expect(store.planPrompt).toBeNull();
  });

  it("回合内无 plan item 不弹窗", async () => {
    await wireEvents();
    store.itemsByThread["t1"] = [
      { id: "m1", type: "agentMessage", text: "没有计划", status: "completed" },
    ];

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-1", status: "completed" },
    });
    await vi.waitFor(() => expect(store.turnActive).toBe(false), {
      timeout: 3000,
      interval: 20,
    });
    expect(store.planPrompt).toBeNull();
  });

  it("非计划模式回合完成不弹窗", async () => {
    await wireEvents();
    store.taskMode = "execute";
    store.itemsByThread["t1"] = [
      { id: "p1", type: "plan", text: "# 计划", status: "completed" },
    ];

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-1", status: "completed" },
    });
    await vi.waitFor(() => expect(store.turnActive).toBe(false), {
      timeout: 3000,
      interval: 20,
    });
    expect(store.planPrompt).toBeNull();
  });

  it("新回合开始（turn/started）关闭计划确认弹窗", async () => {
    await wireEvents();
    store.planPrompt = { threadId: "t1", turnId: "turn-1", planText: "# 计划" };

    fireListen("turn/started", { threadId: "t1", turn: { id: "turn-2" } });
    expect(store.planPrompt).toBeNull();
  });

  it("executePlan 发送 PLEASE IMPLEMENT THIS PLAN 消息并切到执行模式", async () => {
    await wireEvents();
    store.planPrompt = { threadId: "t1", turnId: "turn-1", planText: "# 修复\n1. 步骤" };
    store.goalArmed = true;
    store.currentModel = "gpt-5.2-codex"; // 使 turn/start 携带 collaborationMode
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "nt1" } });
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });

    await executePlan();

    expect(store.taskMode).toBe("execute");
    expect(store.planPrompt).toBeNull();
    // 目标勾选被消费：目标=合成消息（含计划全文）
    expect(store.goalArmed).toBe(false);
    expect(store.goalText).toBe("PLEASE IMPLEMENT THIS PLAN:\n# 修复\n1. 步骤");
    expect(store.goalStatus).toBe("active"); // 随 continueTurn 已挂载
    expect(mockedInvoke).toHaveBeenCalledWith("goal_set", {
      threadId: "t1",
      objective: "PLEASE IMPLEMENT THIS PLAN:\n# 修复\n1. 步骤",
    });
    const call = mockedInvoke.mock.calls.find(([c]) => c === "turn_start");
    expect(call).toBeTruthy();
    const params = (call![1] as { params: Record<string, unknown> }).params;
    expect(params.threadId).toBe("t1");
    expect(params.collaborationMode).toMatchObject({ mode: "default" });
    const input = params.input as { type: string; text: string }[];
    expect(input[0].text.startsWith("PLEASE IMPLEMENT THIS PLAN:\n# 修复\n1. 步骤")).toBe(true);
  });

  it("dismissPlanPrompt 保持计划模式、exitPlanMode 切回执行", () => {
    store.planPrompt = { threadId: "t1", turnId: "turn-1", planText: "# 计划" };
    dismissPlanPrompt();
    expect(store.planPrompt).toBeNull();
    expect(store.taskMode).toBe("plan");

    store.planPrompt = { threadId: "t1", turnId: "turn-1", planText: "# 计划" };
    exitPlanMode();
    expect(store.planPrompt).toBeNull();
    expect(store.taskMode).toBe("execute");
  });
});

describe("历史全量加载（逐页拉取）", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    store.threads = [];
    store.searchActive = false;
    store.searchSnippets = {};
    store.loadingHistory = false;
    store.toast = "";
  });

  it("refreshThreads 逐页累加直至 cursor 为空", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd !== "thread_list") return Promise.resolve(undefined);
      const cursor = (args as { cursor?: string | null } | undefined)?.cursor;
      if (!cursor) {
        return Promise.resolve({
          data: [
            { id: "t1", name: "会话一", createdAt: 0, recencyAt: 0 },
            { id: "t2", name: "会话二", createdAt: 0, recencyAt: 0 },
          ],
          nextCursor: "page2",
        });
      }
      return Promise.resolve({
        data: [{ id: "t3", name: "会话三", createdAt: 0, recencyAt: 0 }],
        nextCursor: null,
      });
    });

    await refreshThreads();

    expect(mockedInvoke).toHaveBeenCalledTimes(2);
    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "thread_list", {
      limit: 50,
      cursor: null,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "thread_list", {
      limit: 50,
      cursor: "page2",
    });
    expect(store.threads.map((t) => t.id)).toEqual(["t1", "t2", "t3"]);
  });

  it("首页为空时立即停止，避免死循环", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd !== "thread_list") return Promise.resolve(undefined);
      return Promise.resolve({
        data: [],
        nextCursor: "page2",
      });
    });

    await refreshThreads();

    expect(mockedInvoke).toHaveBeenCalledTimes(1);
    expect(store.threads).toEqual([]);
  });

  it("searchThreads 逐页累加并汇总摘要", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd !== "codex_rpc") return Promise.resolve(undefined);
      const cursor = (args as { params?: { cursor?: string | null } })
        ?.params?.cursor;
      if (!cursor) {
        return Promise.resolve({
          data: [
            {
              thread: { id: "t1", name: "会话一", createdAt: 0, recencyAt: 0 },
              snippet: "摘要一",
            },
          ],
          nextCursor: "page2",
        });
      }
      return Promise.resolve({
        data: [
          {
            thread: { id: "t2", name: "会话二", createdAt: 0, recencyAt: 0 },
            snippet: "摘要二",
          },
        ],
        nextCursor: null,
      });
    });

    await searchThreads("测试");

    expect(mockedInvoke).toHaveBeenCalledTimes(2);
    expect(store.threads.map((t) => t.id)).toEqual(["t1", "t2"]);
    expect(store.searchSnippets).toEqual({ t1: "摘要一", t2: "摘要二" });
    expect(store.searchActive).toBe(true);
    const searchCalls = mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "codex_rpc",
    );
    expect(searchCalls[0][1]).toMatchObject({
      method: "thread/search",
      params: { searchTerm: "测试", limit: 50, cursor: null },
    });
    expect(searchCalls[1][1]).toMatchObject({
      method: "thread/search",
      params: { searchTerm: "测试", limit: 50, cursor: "page2" },
    });
  });
});

describe("loadSettings 默认权限初始值", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    store.permissionMode = "ask-for-approval";
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
    expect(store.permissionMode).toBe("full-access");
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
    expect(store.permissionMode).toBe("ask-for-approval");
  });
});

describe("loadSettings 记忆模式", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    store.permissionMode = "ask-for-approval";
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
    store.currentThreadId = null;
    store.currentThreadName = "";
    store.currentThreadWorkspace = null;
    store.turnActive = false;
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
    expect(store.currentThreadId).toBe("t1");
    expect(store.currentThreadName).toBe("会话一");
    expect(store.currentThreadWorkspace).toBe("D:/a");
    expect(store.turnActive).toBe(true);

    activeTabId.value = "s2";
    expect(store.currentThreadId).toBe("t2");
    expect(store.currentThreadName).toBe("会话二");
    expect(store.currentThreadWorkspace).toBe("D:/b");
    expect(store.turnActive).toBe(false);
  });

  it("切到文件标签：live 字段保持最近会话不变，切回后仍一致", () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "会话一" }));
    activeTabId.value = "s1";
    expect(store.currentThreadId).toBe("t1");

    (_tabs as unknown as Tab[]).push(makeFileTabFixture("file:test"));
    activeTabId.value = "file:test";
    expect(store.currentThreadId).toBe("t1");
    expect(store.currentThreadName).toBe("会话一");

    activeTabId.value = "s1";
    expect(store.currentThreadId).toBe("t1");
  });

  it("关闭活动会话：投影切到相邻会话", async () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "会话一" }));
    tabs.push(makeSessionTab("s2", "t2", { name: "会话二" }));
    activeTabId.value = "s1";
    expect(store.currentThreadId).toBe("t1");

    await closeSessionTab("s1");
    expect(activeTabId.value).toBe("s2");
    expect(store.currentThreadId).toBe("t2");
    expect(store.currentThreadName).toBe("会话二");
  });

  it("关闭最后一个会话：live 复位默认态，无活动标签", async () => {
    tabs.push(makeSessionTab("s1", "t1", { name: "会话一" }));
    activeTabId.value = "s1";

    await closeSessionTab("s1");
    expect(activeTabId.value).toBe("");
    expect(store.currentThreadId).toBeNull();
    expect(store.currentThreadName).toBe("");
    expect(store.turnActive).toBe(false);
  });
});
