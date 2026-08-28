import { dismissPlanPrompt, executePlan, exitPlanMode } from "../useCodex/actions";
import { handleDynamicToolCall } from "../useCodex/dynamicToolCall";
import { disposeEvents, wireEvents } from "../useCodex/events";
import { __resetSessionTabsForTest, activeSessionTab } from "../useCodex/sessionState";
import { backgroundThreadIds, store } from "../useCodex/store";
import { autoTitleThread } from "../useCodex/threads";
import type { SessionTab } from "../useCodex/types";
import { activeTabId } from "../useEditorTabs";
import { capturedListeners, DEFAULT_MODEL, fireListen, makeSessionTab, mockListenCapture, resetUseCodexState, tabs } from "./useCodexTestHarness";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { flushPromises } from "@vue/test-utils";
import { reactive } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TabIcon, TabKind } from "../../lib/tabs";

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

describe("thread/tokenUsage/updated 记录会话累计输入/输出", () => {
  it("按 threadId 写入归属标签的 input/output，并保留 used/window", async () => {
    __resetSessionTabsForTest();
    const tab = reactive(makeSessionTab("s1", "t1"));
    tabs.push(tab);
    await wireEvents();

    fireListen("thread/tokenUsage/updated", {
      threadId: "t1",
      tokenUsage: {
        total: { totalTokens: 1500, inputTokens: 1000, outputTokens: 500 },
        last: { totalTokens: 900 },
        modelContextWindow: 128000,
      },
    });

    expect(tab.threadTokenUsage).toEqual({
      used: 900,
      window: 128000,
      input: 1000,
      output: 500,
    });
  });

  it("total 缺输入/输出时不写入，保持 used/window", async () => {
    __resetSessionTabsForTest();
    const tab = reactive(makeSessionTab("s1", "t1"));
    tabs.push(tab);
    await wireEvents();

    fireListen("thread/tokenUsage/updated", {
      threadId: "t1",
      tokenUsage: {
        total: { totalTokens: 1500 },
        last: { totalTokens: 900 },
        modelContextWindow: 128000,
      },
    });

    expect(tab.threadTokenUsage).toEqual({ used: 900, window: 128000 });
  });
});

describe("任务栏进度条跟随工作标签", () => {
  beforeEach(() => {
    mockWin.setProgressBar.mockClear();
  });

  afterEach(() => {
    // __resetSessionTabsForTest 只清理会话标签；终端等编辑器标签需显式清空，
    // 避免残留污染后续依赖 tabs[0] 的用例
    tabs.splice(0, tabs.length);
  });

  it("会话回合进行中（含非活动标签）→ Indeterminate，结束后 None", async () => {
    const tab = reactive(makeSessionTab("s1", "t1", { turnActive: true }));
    tabs.push(tab);
    await flushPromises();
    expect(mockWin.setProgressBar).toHaveBeenLastCalledWith({
      status: "Indeterminate",
    });

    tab.turnActive = false;
    await flushPromises();
    expect(mockWin.setProgressBar).toHaveBeenLastCalledWith({ status: "None" });
  });

  it("目标激活（goalStatus active）→ Indeterminate", async () => {
    tabs.push(
      reactive(
        makeSessionTab("s1", "t1", {
          goalStatus: "active",
          goalArmed: true,
        }),
      ),
    );
    await flushPromises();
    expect(mockWin.setProgressBar).toHaveBeenLastCalledWith({
      status: "Indeterminate",
    });
  });

  it("终端命令执行中 → Indeterminate，结束后 None", async () => {
    const terminal = reactive({
      id: "term1",
      kind: TabKind.Terminal,
      title: "终端 (cmd)",
      icon: TabIcon.Terminal,
      workspace: "D:/repo",
      loading: false,
      busy: true,
      exited: false,
      error: "",
    });
    tabs.push(terminal as unknown as SessionTab);
    await flushPromises();
    expect(mockWin.setProgressBar).toHaveBeenLastCalledWith({
      status: "Indeterminate",
    });

    terminal.busy = false;
    await flushPromises();
    expect(mockWin.setProgressBar).toHaveBeenLastCalledWith({ status: "None" });
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


  it("thread/name/updated 更新会话标签名并更新窗口标题", async () => {
    disposeEvents();
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockResolvedValue(undefined);
    __resetSessionTabsForTest();
    tabs.push(reactive({
      id: "s1",
      kind: "chat",
      title: "",
      icon: "chat",
      threadId: "t1",
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      taskMode: "default",
      model: null,
      effort: null,
      plugins: { plugins: [], loaded: false },
      skills: { skills: [], loaded: false },
      creatingChat: false,
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
      plan: null,
      loading: false,
      newChatWorkspace: null,
      interactions: [],
    }));
    activeTabId.value = "s1";
    await wireEvents();
    fireListen("thread/name/updated", {
      threadId: "t1",
      threadName: "新名",
    });
    expect(tabs[0].name).toBe("新名");
    expect(activeSessionTab()?.name).toBe("新名");
    await flushPromises();
    expect(mockWin.setTitle).toHaveBeenCalledWith("repo / 新名");
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

describe("turn/plan/updated 与推理/MCP 增量事件", () => {
  beforeEach(() => {
    disposeEvents();
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
    store.itemsByThread = {};
    backgroundThreadIds.clear();
  });

  it("turn/plan/updated：按 threadId 写入 plan，非法 status 归一化为 pending", async () => {
    tabs.push(makeSessionTab("s1", "t1", { currentTurnId: "turn-1" }));
    activeTabId.value = "s1";
    await wireEvents();

    fireListen("turn/plan/updated", {
      threadId: "t1",
      explanation: "先做 A",
      plan: [
        { step: "A", status: "inProgress" },
        { step: "B", status: "completed" },
        { step: "C", status: "bogus" },
      ],
    });

    expect(tabs[0].plan).toEqual({
      explanation: "先做 A",
      steps: [
        { step: "A", status: "inProgress" },
        { step: "B", status: "completed" },
        { step: "C", status: "pending" },
      ],
    });
  });

  it("turn/plan/updated：仅带 turnId 时按回合归因", async () => {
    tabs.push(makeSessionTab("s1", "t1", { currentTurnId: "turn-9" }));
    activeTabId.value = "s1";
    await wireEvents();

    fireListen("turn/plan/updated", {
      turnId: "turn-9",
      plan: [{ step: "X", status: "pending" }],
    });

    expect(tabs[0].plan?.steps).toEqual([{ step: "X", status: "pending" }]);
  });

  it("turn/plan/updated：无法归因时跳过", async () => {
    tabs.push(makeSessionTab("s1", "t1", { currentTurnId: "turn-1" }));
    activeTabId.value = "s1";
    await wireEvents();

    fireListen("turn/plan/updated", {
      plan: [{ step: "X", status: "pending" }],
    });

    expect(tabs[0].plan).toBeNull();
  });

  it("turn/plan/updated：后台线程跳过", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    backgroundThreadIds.add("bg-1");
    await wireEvents();

    fireListen("turn/plan/updated", {
      threadId: "bg-1",
      plan: [{ step: "X", status: "pending" }],
    });

    expect(tabs[0].plan).toBeNull();
  });

  it("turn/started 重置 plan", async () => {
    tabs.push(makeSessionTab("s1", "t1", { currentTurnId: "turn-1" }));
    activeTabId.value = "s1";
    await wireEvents();
    fireListen("turn/plan/updated", {
      threadId: "t1",
      plan: [{ step: "A", status: "inProgress" }],
    });
    expect(tabs[0].plan).not.toBeNull();

    fireListen("turn/started", { threadId: "t1", turn: { id: "turn-2" } });
    expect(tabs[0].plan).toBeNull();
  });

  it("item/reasoning/summaryTextDelta 按索引追加，summaryPartAdded 扩展数组", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    await wireEvents();

    fireListen("item/reasoning/summaryTextDelta", {
      threadId: "t1",
      itemId: "r1",
      summaryIndex: 0,
      delta: "思考",
    });
    fireListen("item/reasoning/summaryTextDelta", {
      threadId: "t1",
      itemId: "r1",
      summaryIndex: 0,
      delta: "中",
    });
    fireListen("item/reasoning/summaryPartAdded", {
      threadId: "t1",
      itemId: "r1",
      summaryIndex: 2,
    });

    const items = store.itemsByThread["t1"] ?? [];
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({
      type: "reasoning",
      summary: ["思考中", "", ""],
      streaming: true,
    });
  });

  it("item/mcpToolCall/progress 写入进度字段，缺失字段容错", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    await wireEvents();

    fireListen("item/mcpToolCall/progress", {
      threadId: "t1",
      itemId: "m1",
      message: "下载中",
      percent: 50,
    });
    let items = store.itemsByThread["t1"] ?? [];
    expect(items[0]).toMatchObject({
      type: "mcpToolCall",
      progressText: "下载中",
      progressPercent: 50,
    });

    // 无 message/percent 的后续事件不覆盖已有值
    fireListen("item/mcpToolCall/progress", { threadId: "t1", itemId: "m1" });
    items = store.itemsByThread["t1"] ?? [];
    expect(items[0]).toMatchObject({
      progressText: "下载中",
      progressPercent: 50,
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
    await wireEvents();
    fireListen("turn/completed", {
      threadId: "t2",
      turn: { id: "turn-2", status: "completed" },
    });
    expect(tabs[1].turnActive).toBe(false);
    expect(tabs[0].turnActive).toBe(true);
    expect(activeSessionTab()?.turnActive).toBe(true);
    disposeEvents();
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
    store.models = [DEFAULT_MODEL];
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
    expect(activeSessionTab()?.turnActive).toBe(true);
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
    tabs.push(
      makeSessionTab("s1", "t1", {
        taskMode: "plan",
        goalText: "发布 v2",
        goalStatus: "active",
      }),
    );
    activeTabId.value = "s1";
  });

  it("thread/goal/updated：同步当前会话的目标文本与状态", async () => {
    await wireEvents();
    fireListen("thread/goal/updated", {
      threadId: "t1",
      goal: { objective: "发布 v2", status: "active" },
    });
    expect(activeSessionTab()?.goalText).toBe("发布 v2");
    expect(activeSessionTab()?.goalStatus).toBe("active");
  });

  it("thread/goal/updated 终态：自动清目标并复位 flag（goal_clear）", async () => {
    await wireEvents();
    store.toast = "";
    fireListen("thread/goal/updated", {
      threadId: "t1",
      goal: { objective: "发布 v2", status: "complete" },
    });
    await vi.waitFor(() => expect(activeSessionTab()?.goalText).toBeNull(), {
      timeout: 3000,
      interval: 20,
    });
    expect(activeSessionTab()?.goalStatus).toBeNull();
    expect(activeSessionTab()?.goalArmed).toBe(false);
    expect(store.toast).toContain("目标已完成");
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", { threadId: "t1" });
  });

  it("thread/goal/updated：旧会话通知不污染当前会话", async () => {
    await wireEvents();
    tabs[0].goalText = "当前目标";
    tabs[0].goalStatus = "active";
    fireListen("thread/goal/updated", {
      threadId: "t-other",
      goal: { objective: "别人", status: "complete" },
    });
    expect(activeSessionTab()?.goalText).toBe("当前目标");
    expect(activeSessionTab()?.goalStatus).toBe("active");
  });

  it("thread/goal/cleared：清空当前会话目标", async () => {
    await wireEvents();
    fireListen("thread/goal/cleared", { threadId: "t1" });
    expect(activeSessionTab()?.goalText).toBeNull();
    expect(activeSessionTab()?.goalStatus).toBeNull();
    expect(activeSessionTab()?.goalArmed).toBe(false);
  });

  it("turn/completed：不再自动清除目标（目标由服务端循环管理）", async () => {
    await wireEvents();
    store.itemsByThread["t1"] = [];
    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-1", status: "completed" },
    });
    await vi.waitFor(() => expect(activeSessionTab()?.turnActive).toBe(false), {
      timeout: 3000,
      interval: 20,
    });
    expect(activeSessionTab()?.goalText).toBe("发布 v2");
    expect(activeSessionTab()?.goalStatus).toBe("active");
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
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1", { taskMode: "plan" }));
    activeTabId.value = "s1";
    store.itemsRev = 0;
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
    await vi.waitFor(() => expect(activeSessionTab()?.turnActive).toBe(false), {
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
    await vi.waitFor(() => expect(activeSessionTab()?.turnActive).toBe(false), {
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
    tabs.push(makeSessionTab("s2", "t2"));
    activeTabId.value = "s2";
    store.itemsByThread = {};
    store.activeWorkByThread = {};
  });

  afterEach(() => {
    disposeEvents();
  });

  it("旧线程 turn/completed：不复位状态、不弹计划确认、不消费队列", async () => {
    await wireEvents();
    tabs[0].turnActive = true; // 模拟新会话正在运行
    tabs[0].followupQueue.push({ text: "队列消息", attachments: [] });
    store.itemsByThread["t1"] = [
      { id: "p1", type: "plan", text: "旧会话计划", status: "completed" },
    ];

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "old-turn", status: "completed" },
    });
    await flushPromises();

    expect(tabs[0].turnActive).toBe(true);
    expect(tabs[0].currentTurnId).toBeNull();
    expect(tabs[0].planPrompt).toBeNull();
    expect(tabs[0].followupQueue).toHaveLength(1);
  });

  it("旧线程 turn/started：不把新会话置为进行中", async () => {
    await wireEvents();
    fireListen("turn/started", { threadId: "t1", turn: { id: "old-turn" } });
    expect(tabs[0].turnActive).toBe(false);
    expect(tabs[0].currentTurnId).toBeNull();
  });
});
describe("后台临时线程 delta 事件隔离", () => {
  const LONG_TEXT = "这是一个非常长的用户消息，用来验证标题总结功能能否正常触发和写回。".repeat(2);

  beforeEach(() => {
    disposeEvents();
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    store.toast = "";
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
    tabs.push(makeSessionTab("s1", "t1", { taskMode: "plan" }));
    activeTabId.value = "s1";
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
    await vi.waitFor(() => expect(activeSessionTab()?.planPrompt).not.toBeNull(), {
      timeout: 3000,
      interval: 20,
    });

    expect(activeSessionTab()?.planPrompt).toEqual({
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
    await vi.waitFor(() => expect(activeSessionTab()?.planPrompt?.planText).toBe("新计划"), {
      timeout: 3000,
      interval: 20,
    });
  });

  it("第二轮评估未产出 plan：不再弹旧计划（回归：跨回合误扫）", async () => {
    await wireEvents();
    store.itemsByThread["t1"] = [
      { id: "u1", type: "userMessage", content: [], startedAtMs: 1000 },
      {
        id: "p1",
        type: "plan",
        text: "计划A",
        status: "completed",
        startedAtMs: 2000,
      },
      { id: "u2", type: "userMessage", content: [], startedAtMs: 3000 },
    ];

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-2", status: "completed" },
    });
    await flushPromises();
    expect(activeSessionTab()?.planPrompt).toBeNull();
  });

  it("第二轮真正产出 plan：弹出新计划而非旧计划", async () => {
    await wireEvents();
    store.itemsByThread["t1"] = [
      { id: "u1", type: "userMessage", content: [], startedAtMs: 1000 },
      {
        id: "p1",
        type: "plan",
        text: "计划A",
        status: "completed",
        startedAtMs: 2000,
      },
      { id: "u2", type: "userMessage", content: [], startedAtMs: 3000 },
      {
        id: "p2",
        type: "plan",
        text: "计划B",
        status: "completed",
        startedAtMs: 4000,
      },
    ];

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-2", status: "completed" },
    });
    await flushPromises();
    expect(activeSessionTab()?.planPrompt).toEqual({
      threadId: "t1",
      turnId: "turn-2",
      planText: "计划B",
    });
  });

  it("组合场景：turn/started 清空旧提示，第二轮无新 plan 不弹", async () => {
    await wireEvents();
    (tabs[0] as SessionTab).planPrompt = {
      threadId: "t1",
      turnId: "turn-1",
      planText: "计划A",
    };
    store.itemsByThread["t1"] = [
      { id: "u1", type: "userMessage", content: [], startedAtMs: 1000 },
      {
        id: "p1",
        type: "plan",
        text: "计划A",
        status: "completed",
        startedAtMs: 2000,
      },
      { id: "u2", type: "userMessage", content: [], startedAtMs: 3000 },
    ];

    fireListen("turn/started", { threadId: "t1", turn: { id: "turn-2" } });
    expect(activeSessionTab()?.planPrompt).toBeNull();

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-2", status: "completed" },
    });
    await flushPromises();
    expect(activeSessionTab()?.planPrompt).toBeNull();
  });

  it("tab 为 plan、store 为 default：仍按标签模式弹出计划确认（不读全局 taskMode）", async () => {
    await wireEvents();
    store.itemsByThread["t1"] = [
      { id: "p1", type: "plan", text: "计划A", status: "completed" },
    ];
    (tabs[0] as SessionTab).taskMode = "plan";

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-1", status: "completed" },
    });
    expect(activeSessionTab()?.planPrompt).toEqual({
      threadId: "t1",
      turnId: "turn-1",
      planText: "计划A",
    });
  });

  it("tab 为 default、store 为 plan：不弹计划提示（计划判定按标签模式）", async () => {
    await wireEvents();
    store.itemsByThread["t1"] = [
      { id: "p1", type: "plan", text: "计划A", status: "completed" },
    ];
    (tabs[0] as SessionTab).taskMode = "default";

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-1", status: "completed" },
    });
    expect(activeSessionTab()?.planPrompt).toBeNull();
    expect((tabs[0] as SessionTab).planPrompt).toBeNull();
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
    await vi.waitFor(() => expect(activeSessionTab()?.turnActive).toBe(false), {
      timeout: 3000,
      interval: 20,
    });
    expect(activeSessionTab()?.planPrompt).toBeNull();
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
    await vi.waitFor(() => expect(activeSessionTab()?.turnActive).toBe(false), {
      timeout: 3000,
      interval: 20,
    });
    expect(activeSessionTab()?.planPrompt).toBeNull();
  });

  it("非计划模式回合完成不弹窗", async () => {
    await wireEvents();
    (tabs[0] as SessionTab).taskMode = "default";
    store.itemsByThread["t1"] = [
      { id: "p1", type: "plan", text: "# 计划", status: "completed" },
    ];

    fireListen("turn/completed", {
      threadId: "t1",
      turn: { id: "turn-1", status: "completed" },
    });
    await vi.waitFor(() => expect(activeSessionTab()?.turnActive).toBe(false), {
      timeout: 3000,
      interval: 20,
    });
    expect(activeSessionTab()?.planPrompt).toBeNull();
  });

  it("新回合开始（turn/started）关闭计划确认弹窗", async () => {
    await wireEvents();

    fireListen("turn/started", { threadId: "t1", turn: { id: "turn-2" } });
    expect(activeSessionTab()?.planPrompt).toBeNull();
  });

  it("executePlan 发送 PLEASE IMPLEMENT THIS PLAN 消息并切到执行模式", async () => {
    await wireEvents();
    tabs[0].planPrompt = {
      threadId: "t1",
      turnId: "turn-1",
      planText: "# 修复\n1. 步骤",
    };
    tabs[0].goalArmed = true;
    tabs[0].model = "gpt-5.2-codex"; // 使 turn/start 携带 collaborationMode
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "turn_start") return Promise.resolve({ turn: { id: "nt1" } });
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });

    await executePlan();

    expect(activeSessionTab()?.taskMode).toBe("default");
    expect(activeSessionTab()?.planPrompt).toBeNull();
    // 目标勾选被消费：目标=合成消息（含计划全文）
    expect(activeSessionTab()?.goalArmed).toBe(false);
    expect(activeSessionTab()?.goalText).toBe("PLEASE IMPLEMENT THIS PLAN:\n# 修复\n1. 步骤");
    expect(activeSessionTab()?.goalStatus).toBe("active"); // 随 continueTurn 已挂载
    expect(mockedInvoke).toHaveBeenCalledWith("goal_set", {
      threadId: "t1",
      objective: "PLEASE IMPLEMENT THIS PLAN:\n# 修复\n1. 步骤",
    });
    const call = mockedInvoke.mock.calls.find(([c]) => c === "turn_start");
    expect(call).toBeTruthy();
    const params = (call![1] as { params: Record<string, unknown> }).params;
    expect(params.threadId).toBe("t1");
    expect(params.collaborationMode).toMatchObject({ mode: "default" });
    expect(mockedInvoke).toHaveBeenCalledWith(
      "session_log",
      expect.objectContaining({
        event: "turn-start-mode",
        detail: "collaborationMode.mode=default",
      }),
    );
    const input = params.input as { type: string; text: string }[];
    expect(input[0].text.startsWith("PLEASE IMPLEMENT THIS PLAN:\n# 修复\n1. 步骤")).toBe(true);
  });

  it("dismissPlanPrompt 保持计划模式、exitPlanMode 切回执行", () => {
    tabs[0].planPrompt = { threadId: "t1", turnId: "turn-1", planText: "# 计划" };
    dismissPlanPrompt();
    expect(activeSessionTab()?.planPrompt).toBeNull();
    expect(activeSessionTab()?.taskMode).toBe("plan");

    tabs[0].planPrompt = { threadId: "t1", turnId: "turn-1", planText: "# 计划" };
    exitPlanMode();
    expect(activeSessionTab()?.planPrompt).toBeNull();
    expect(activeSessionTab()?.taskMode).toBe("default");
  });
});

describe("事件路由：缺 threadId 时按回合 id 归因（不回退活动会话）", () => {
  beforeEach(() => {
    disposeEvents();
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
    store.itemsByThread = {};
    store.activeWorkByThread = {};
  });

  afterEach(() => {
    disposeEvents();
  });

  it("turn/completed 缺 threadId 且 turn id 匹配后台标签 B：只写 B，活动标签 A 不受影响", async () => {
    tabs.push(
      makeSessionTab("sA", "tA", {
        turnActive: true,
        currentTurnId: "turn-a",
        taskMode: "default",
      }),
    );
    tabs.push(
      makeSessionTab("sB", "tB", {
        turnActive: true,
        currentTurnId: "turn-b",
        taskMode: "plan",
      }),
    );
    activeTabId.value = "sA";
    store.itemsByThread["tB"] = [
      { id: "p1", type: "plan", text: "B的计划", status: "completed" },
    ];
    await wireEvents();

    fireListen("turn/completed", {
      turn: { id: "turn-b", status: "completed" },
    });
    await flushPromises();

    expect((tabs[1] as SessionTab).turnActive).toBe(false);
    expect((tabs[1] as SessionTab).planPrompt).toEqual({
      threadId: "tB",
      turnId: "turn-b",
      planText: "B的计划",
    });
    // 活动标签 A 的回合状态与提示均不受影响
    expect((tabs[0] as SessionTab).turnActive).toBe(true);
    expect(activeSessionTab()?.turnActive).toBe(true);
    expect(activeSessionTab()?.planPrompt).toBeNull();
  });

  it("turn/completed 缺 threadId 且 turn id 匹配活动会话：正常写 store", async () => {
    tabs.push(
      makeSessionTab("sA", "tA", {
        turnActive: true,
        currentTurnId: "turn-a",
        taskMode: "plan",
      }),
    );
    activeTabId.value = "sA";
    store.itemsByThread["tA"] = [
      { id: "p1", type: "plan", text: "A的计划", status: "completed" },
    ];
    await wireEvents();

    fireListen("turn/completed", {
      turn: { id: "turn-a", status: "completed" },
    });
    await flushPromises();

    expect(activeSessionTab()?.turnActive).toBe(false);
    expect(activeSessionTab()?.planPrompt).toEqual({
      threadId: "tA",
      turnId: "turn-a",
      planText: "A的计划",
    });
  });

  it("turn/completed 缺 threadId 且无任何标签匹配 turn id：跳过，活动标签状态不变", async () => {
    tabs.push(
      makeSessionTab("sA", "tA", {
        turnActive: true,
        currentTurnId: "turn-a",
        taskMode: "plan",
      }),
    );
    activeTabId.value = "sA";
    await wireEvents();

    fireListen("turn/completed", {
      turn: { id: "turn-zzz", status: "completed" },
    });
    await flushPromises();

    expect(activeSessionTab()?.turnActive).toBe(true);
    expect(activeSessionTab()?.planPrompt).toBeNull();
    expect((tabs[0] as SessionTab).turnActive).toBe(true);
  });

  it("turn/started 缺 threadId 且 turn id 匹配后台标签 B：B 置为进行中，A 的计划提示不被清空", async () => {
    tabs.push(
      makeSessionTab("sA", "tA", {
        turnActive: false,
        currentTurnId: "turn-a",
        taskMode: "plan",
        planPrompt: { threadId: "tA", turnId: "turn-1", planText: "旧提示" },
      }),
    );
    tabs.push(
      makeSessionTab("sB", "tB", {
        turnActive: false,
        currentTurnId: "turn-b",
        taskMode: "default",
      }),
    );
    activeTabId.value = "sA";
    await wireEvents();

    fireListen("turn/started", { turn: { id: "turn-b" } });
    await flushPromises();

    expect((tabs[1] as SessionTab).turnActive).toBe(true);
    // A 的“计划已就绪”提示不被 B 的回合开始事件清掉
    expect(activeSessionTab()?.planPrompt).not.toBeNull();
    expect((tabs[0] as SessionTab).planPrompt).not.toBeNull();
  });

  it("thread/goal/updated 与 cleared 缺 threadId：跳过，活动标签目标状态不变", async () => {
    tabs.push(
      makeSessionTab("sA", "tA", {
        goalText: "A目标",
        goalStatus: "active",
        goalArmed: true,
      }),
    );
    activeTabId.value = "sA";
    await wireEvents();

    fireListen("thread/goal/updated", {
      goal: { objective: "B目标", status: "active" },
    });
    fireListen("thread/goal/cleared", {});
    await flushPromises();

    expect(activeSessionTab()?.goalText).toBe("A目标");
    expect(activeSessionTab()?.goalStatus).toBe("active");
    expect(activeSessionTab()?.goalArmed).toBe(true);
    expect((tabs[0] as SessionTab).goalText).toBe("A目标");
  });
});

describe("thread/settings/updated 服务端任务模式对账", () => {
  beforeEach(() => {
    disposeEvents();
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
  });

  it("服务端 default：plan 标签 taskMode 切到 default 并记录 warn", async () => {
    tabs.push(makeSessionTab("s1", "t1", { taskMode: "plan" }));
    await wireEvents();

    fireListen("thread/settings/updated", {
      threadId: "t1",
      threadSettings: { collaborationMode: { mode: "default" } },
    });
    await flushPromises();

    expect((tabs[0] as SessionTab).taskMode).toBe("default");
    expect(mockedInvoke).toHaveBeenCalledWith(
      "session_log",
      expect.objectContaining({
        level: "warn",
        event: "task-mode-reconcile",
      }),
    );
  });

  it("服务端 plan：default 标签 taskMode 切到 plan", async () => {
    tabs.push(makeSessionTab("s1", "t1", { taskMode: "default" }));
    await wireEvents();

    fireListen("thread/settings/updated", {
      threadId: "t1",
      threadSettings: { collaborationMode: { mode: "plan" } },
    });
    await flushPromises();

    expect((tabs[0] as SessionTab).taskMode).toBe("plan");
  });

  it("字段缺失/未知取值：不影响本地模式且不抛错", async () => {
    tabs.push(makeSessionTab("s1", "t1", { taskMode: "plan" }));
    await wireEvents();

    fireListen("thread/settings/updated", {
      threadId: "t1",
      threadSettings: {},
    });
    fireListen("thread/settings/updated", {
      threadId: "t1",
      threadSettings: { collaborationMode: { mode: "bogus" } },
    });
    await flushPromises();

    expect((tabs[0] as SessionTab).taskMode).toBe("plan");
  });
});

describe("codexui 动态工具 item/tool/call 应答", () => {
  it("get_usage 用会话用量拼文本并应答，不进入交互气泡", async () => {
    tabs.push(
      reactive(
        makeSessionTab("s1", "t1", {
          threadTokenUsage: {
            used: 12000,
            window: 128000,
            input: 50000,
            output: 9000,
          },
        }),
      ),
    );

    await handleDynamicToolCall({
      requestId: 7,
      method: "item/tool/call",
      params: {
        threadId: "t1",
        turnId: "t1",
        callId: "c1",
        namespace: "codexui",
        tool: "get_usage",
        arguments: {},
      },
    });

    expect(mockedInvoke).toHaveBeenCalledWith("interaction_respond", {
      requestId: 7,
      result: {
        contentItems: [
          {
            type: "inputText",
            text: "上下文已用 12K/128K（约 9%）；会话累计输入 50K · 输出 9K",
          },
        ],
        success: true,
      },
    });
    expect(tabs[0].interactions).toHaveLength(0);
  });

  it("compact_context 发起 thread/compact/start 并应答 success true", async () => {
    await handleDynamicToolCall({
      requestId: 8,
      method: "item/tool/call",
      params: {
        threadId: "t1",
        turnId: "",
        callId: "c2",
        namespace: "codexui",
        tool: "compact_context",
        arguments: {},
      },
    });

    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/compact/start",
      params: { threadId: "t1" },
    });
    expect(mockedInvoke).toHaveBeenCalledWith("interaction_respond", {
      requestId: 8,
      result: {
        contentItems: [{ type: "inputText", text: "已请求压缩上下文" }],
        success: true,
      },
    });
  });

  it("compact_context 无 threadId 应答 success false", async () => {
    await handleDynamicToolCall({
      requestId: 9,
      method: "item/tool/call",
      params: {
        turnId: "",
        callId: "c3",
        namespace: "codexui",
        tool: "compact_context",
        arguments: {},
      },
    });

    expect(mockedInvoke).toHaveBeenCalledWith("interaction_respond", {
      requestId: 9,
      result: {
        contentItems: [{ type: "inputText", text: "压缩失败：缺少 threadId" }],
        success: false,
      },
    });
  });

  it("未知工具应答 success false", async () => {
    await handleDynamicToolCall({
      requestId: 10,
      method: "item/tool/call",
      params: {
        threadId: "t1",
        turnId: "",
        callId: "c4",
        namespace: "codexui",
        tool: "nope",
        arguments: {},
      },
    });

    expect(mockedInvoke).toHaveBeenCalledWith("interaction_respond", {
      requestId: 10,
      result: expect.objectContaining({ success: false }),
    });
  });
});
