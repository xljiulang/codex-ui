import { dismissPlanPrompt, executePlan, exitPlanMode } from "../useCodex/actions";
import { __resetTitleHelperCapabilityForTest } from "../useCodex/capabilities";
import { disposeEvents, wireEvents } from "../useCodex/events";
import { __resetSessionTabsForTest } from "../useCodex/sessionState";
import { store } from "../useCodex/store";
import { autoTitleThread } from "../useCodex/threads";
import { activeTabId } from "../useEditorTabs";
import { capturedListeners, fireListen, makeSessionTab, mockListenCapture, resetUseCodexState, tabs } from "./useCodexTestHarness";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { flushPromises } from "@vue/test-utils";
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
