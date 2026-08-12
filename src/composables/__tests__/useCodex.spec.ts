import { describe, expect, it, beforeEach, afterEach } from "vitest";
import { vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({
    isMinimized: vi.fn().mockResolvedValue(false),
    unminimize: vi.fn(),
    setFocus: vi.fn(),
    setTitle: vi.fn(),
    setProgressBar: vi.fn(),
  })),
  ProgressBarStatus: { Indeterminate: "Indeterminate", None: "None" },
}));

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  __resetPinnedSectionForTest,
  __resetTitleHelperCapabilityForTest,
  autoTitleThread,
  disposeEvents,
  ensureSkills,
  ensureThreadPlugins,
  interrupt,
  newEmptyChat,
  openThread,
  refreshThreads,
  refreshServer,
  sanitizeTitle,
  searchThreads,
  sendPrompt,
  settleConfirm,
  sortThreads,
  store,
  togglePin,
  toastError,
  wireEvents,
} from "../useCodex";

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
});

describe("refreshServer 服务状态同步", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    store.server = {
      connected: false,
      workspace: "",
      codexPath: null,
      logs: [],
    };
  });

  it("server_status 返回 codexPath 后写入 store.server.codexPath", async () => {
    mockedInvoke.mockResolvedValue({
      connected: true,
      workspace: "D:/repo",
      codexPath: "D:/codex/codex.exe",
      logs: [],
    });
    await refreshServer();
    expect(mockedInvoke).toHaveBeenCalledWith("server_status");
    expect(store.server.codexPath).toBe("D:/codex/codex.exe");
    expect(store.server.connected).toBe(true);
    expect(store.server.workspace).toBe("D:/repo");
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
    await sendPrompt("第二条消息");
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

describe("切换会话自动标准停止旧回合", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    store.threadPlugins = {};
    store.turnActive = false;
    store.turnInterrupted = false;
    store.currentThreadId = null;
    store.currentThreadName = "";
    store.currentTurnId = null;
    store.currentThreadOrigin = null;
    store.currentThreadCwd = null;
    store.resumedThreadId = null;
    store.threadTokenUsage = null;
    store.goalText = null;
    store.taskMode = "execute";
    store.confirm = null;
  });

  it("新建会话时标准停止旧回合（turn_interrupt），再复位到新会话", async () => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    const p = newEmptyChat();
    expect(store.confirm?.title).toBe("切换会话");
    settleConfirm(true);
    await p;
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(store.currentThreadId).toBeNull();
    expect(store.currentTurnId).toBeNull();
  });

  it("目标模式下新建会话：先清旧会话目标，再中断旧回合", async () => {
    store.turnActive = true;
    store.taskMode = "goal";
    store.goalText = "旧目标";
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    const p = newEmptyChat();
    settleConfirm(true);
    await p;
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", {
      threadId: "t1",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(store.currentThreadId).toBeNull();
  });

  it("切换到其他历史会话时标准停止旧回合，并打开目标会话", async () => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
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
    settleConfirm(true);
    await p;
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(store.currentThreadId).toBe("t2");
  });

  it("点击当前正在进行的会话不算切换，不中断", async () => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_read") {
        return Promise.resolve({
          thread: { id: "t1", name: "会话1", turns: [] },
        });
      }
      return Promise.resolve(undefined);
    });
    await openThread("t1");
    const interruptCalls = mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "turn_interrupt",
    );
    expect(interruptCalls).toHaveLength(0);
    expect(store.confirm).toBeNull();
    // 点击当前会话不重载、不重置运行状态
    expect(store.turnActive).toBe(true);
    expect(store.currentTurnId).toBe("turn-1");
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "thread_read",
      expect.anything(),
    );
  });

  it("空闲时点击当前会话同样不重载、不改动任何状态", async () => {
    store.turnActive = false;
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-9";
    store.currentThreadName = "会话一";
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_read") {
        return Promise.resolve({
          thread: { id: "t1", name: "改过的名字", turns: [] },
        });
      }
      return Promise.resolve(undefined);
    });

    await openThread("t1");

    expect(store.currentThreadId).toBe("t1");
    expect(store.currentThreadName).toBe("会话一");
    expect(store.turnActive).toBe(false);
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "thread_read",
      expect.anything(),
    );
  });

  it("会话进行中取消切换：新建会话不中断、不切换", async () => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    const p = newEmptyChat();
    settleConfirm(false);
    await p;
    const interruptCalls = mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "turn_interrupt",
    );
    expect(interruptCalls).toHaveLength(0);
    expect(store.currentThreadId).toBe("t1");
  });

  it("会话进行中取消切换：历史会话不打开、不中断", async () => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    const p = openThread("t2");
    settleConfirm(false);
    await p;
    const interruptCalls = mockedInvoke.mock.calls.filter(
      ([cmd]) => cmd === "turn_interrupt",
    );
    expect(interruptCalls).toHaveLength(0);
    expect(store.currentThreadId).toBe("t1");
  });

  it("目标模式标准停止：先清目标再 turn/interrupt（默认操作当前会话）", async () => {
    store.turnActive = true;
    store.taskMode = "goal";
    store.goalText = "目标";
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    await interrupt();
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", {
      threadId: "t1",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    expect(store.goalText).toBeNull();
    expect(store.taskMode).toBe("execute");
  });

  it("显式传入旧线程/回合 id 时：目标清除作用于旧线程，且不污染新会话的回合 id", async () => {
    store.taskMode = "goal";
    store.goalText = "旧目标";
    store.currentThreadId = "t2"; // 模拟 store 已切到新会话
    store.currentTurnId = null;
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
    await interrupt("t1", "turn-1");
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", {
      threadId: "t1",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "turn-1",
    });
    // 重试用服务端返回的活跃回合 id，但不写进 store（那是新会话的状态）
    expect(mockedInvoke).toHaveBeenCalledWith("turn_interrupt", {
      threadId: "t1",
      turnId: "abc-123",
    });
    expect(store.currentTurnId).toBeNull();
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
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    __resetTitleHelperCapabilityForTest();
    store.toast = "";
    store.currentThreadId = "t1";
    store.currentThreadName = "";
    store.currentThreadCwd = "D:/repo";
    store.server.workspace = "D:/repo";
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
    store.currentThreadName = "手动标题";
    await autoTitleThread("t1", LONG_TEXT);
    expect(mockedInvoke).not.toHaveBeenCalledWith("thread_start", expect.anything());
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
