import { newEmptyChat, openThread, sendPrompt } from "../useCodex/actions";
import { __resetSessionTabsForTest, activeSessionTab } from "../useCodex/sessionState";
import { store } from "../useCodex/store";
import { buildTurnParams, clearGoal, setGoal } from "../useCodex/turnControl";
import { activeTabId } from "../useEditorTabs";
import { DEFAULT_MODEL, makeSessionTab, resetUseCodexState, tabs } from "./useCodexTestHarness";
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

describe("线程级目标：设置/清除/读取/事件同步", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.toast = "";
  });

  it("setGoal：无会话时不挂载（返回 false，不调用 goal_set）", async () => {
    const ok = await setGoal("修复登录");
    expect(ok).toBe(false);
    expect(activeSessionTab()).toBeNull();
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "goal_set",
      expect.anything(),
    );
  });

  it("setGoal：空文本不提交", async () => {
    const ok = await setGoal("   ");
    expect(ok).toBe(false);
    expect(store.toast).toContain("目标不能为空");
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "goal_set",
      expect.anything(),
    );
  });

  it("setGoal：超过 4000 字符被拒绝", async () => {
    const ok = await setGoal("长".repeat(4001));
    expect(ok).toBe(false);
    expect(store.toast).toContain("4000");
  });

  it("setGoal：成功后本地记录目标与 active 状态", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    mockedInvoke.mockResolvedValue(undefined);
    const ok = await setGoal("  修复登录流程  ");
    expect(ok).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("goal_set", {
      threadId: "t1",
      objective: "修复登录流程",
    });
    expect(activeSessionTab()?.goalText).toBe("修复登录流程");
    expect(activeSessionTab()?.goalStatus).toBe("active");
    expect(store.toast).toContain("已设置目标");
  });

  it("setGoal：失败时保留原目标并提示错误", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", {
        goalText: "旧目标",
        goalStatus: "complete",
      }),
    );
    activeTabId.value = "s1";
    mockedInvoke.mockRejectedValue(new Error("服务端拒绝"));
    const ok = await setGoal("新目标");
    expect(ok).toBe(false);
    expect(activeSessionTab()?.goalText).toBe("旧目标");
    expect(activeSessionTab()?.goalStatus).toBe("complete");
    expect(store.toast).toContain("服务端拒绝");
  });

  it("clearGoal：无会话时仅清空本地状态", async () => {
    await clearGoal();
    expect(activeSessionTab()).toBeNull();
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
    expect(activeSessionTab()?.threadId).toBe("t2");
    expect(tabs[0].goalText).toBe("旧目标");
  });

  it("openThread：goal_get 返回终态时 toast + 复位 + goal_clear", async () => {
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
    expect(activeSessionTab()?.threadId).toBe("t2");
    // 终态目标：打开即 toast 提示并复位（相当于没有目标），服务端同步清除
    expect(activeSessionTab()?.goalText).toBeNull();
    expect(activeSessionTab()?.goalStatus).toBeNull();
    expect(activeSessionTab()?.goalArmed).toBe(false);
    expect(store.toast).toContain("目标已完成");
    expect(mockedInvoke).toHaveBeenCalledWith("goal_clear", { threadId: "t2" });
  });

  it("openThread：goal_get 兼容 {goal:{objective,status}} 包裹返回", async () => {
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
    expect(activeSessionTab()?.goalText).toBe("重构登录");
    expect(activeSessionTab()?.goalStatus).toBe("active");
  });

  it("openThread：goal_get 失败或未挂目标时状态为空", async () => {
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
    expect(activeSessionTab()?.goalText).toBeNull();
    expect(activeSessionTab()?.goalStatus).toBeNull();
  });

  it("openThread：loadFullItems 以 asc+full 拉取完整工具/命令详情", async () => {
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

  it("openThread：分页线程以 includeTurns=false 读元数据，消息来自 turns/list", async () => {
    const items = [
      { id: "i1", type: "userMessage", content: [{ type: "inputText", text: "hi" }] },
      { id: "i2", type: "agentMessage", content: [{ type: "outputText", text: "hello" }] },
    ];
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "thread_read") {
        return Promise.resolve({
          thread: {
            id: "t2",
            name: "会话2",
            cwd: "/ws",
            historyMode: "paginated",
            turns: [],
          },
        });
      }
      if (cmd === "codex_rpc") {
        const method = (args as { method?: string })?.method;
        if (method === "thread/turns/list") {
          return Promise.resolve({
            data: [{ id: "turn-1", status: "completed", items }],
            nextCursor: null,
          });
        }
      }
      if (cmd === "goal_get") return Promise.resolve({});
      return Promise.resolve(undefined);
    });
    const p = openThread("t2");
    expect(await p).toBe(true);
    // 分页线程：只读元数据，绝不尝试 includeTurns=true
    expect(mockedInvoke).toHaveBeenCalledWith("thread_read", {
      threadId: "t2",
      includeTurns: false,
    });
    expect(mockedInvoke).not.toHaveBeenCalledWith("thread_read", {
      threadId: "t2",
      includeTurns: true,
    });
    expect(activeSessionTab()?.threadId).toBe("t2");
    expect(activeSessionTab()?.workspace).toBe("/ws");
    expect(store.itemsByThread["t2"]).toEqual(items);
    expect(store.toast).toBe("");
  });

  it("openThread：legacy 线程 turns/list 失败时回退 includeTurns=true 摘要", async () => {
    const summaryItems = [
      { id: "s1", type: "userMessage", content: [{ type: "inputText", text: "旧消息" }] },
    ];
    mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "thread_read") {
        const includeTurns = (args as { includeTurns?: boolean })?.includeTurns;
        if (includeTurns === true) {
          return Promise.resolve({
            thread: {
              id: "t2",
              name: "会话2",
              turns: [
                { id: "turn-1", status: "completed", items: summaryItems },
              ],
            },
          });
        }
        return Promise.resolve({
          thread: { id: "t2", name: "会话2", turns: [] },
        });
      }
      if (cmd === "codex_rpc") {
        const method = (args as { method?: string })?.method;
        if (method === "thread/turns/list") {
          return Promise.reject(new Error("method not found"));
        }
      }
      if (cmd === "goal_get") return Promise.resolve({});
      return Promise.resolve(undefined);
    });
    const p = openThread("t2");
    expect(await p).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("thread_read", {
      threadId: "t2",
      includeTurns: false,
    });
    expect(mockedInvoke).toHaveBeenCalledWith("thread_read", {
      threadId: "t2",
      includeTurns: true,
    });
    expect(activeSessionTab()?.threadId).toBe("t2");
    expect(store.itemsByThread["t2"]).toEqual(summaryItems);
  });

  it("continueTurn：待挂载目标在回合启动前 goal_set 挂载", async () => {
    tabs.push(
      makeSessionTab("s1", "t1", {
        goalText: "修复登录",
        goalStatus: null,
      }),
    );
    activeTabId.value = "s1";
    tabs[0].resumedThreadId = "t1"; // 跳过 thread_resume
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "turn_start") {
        return Promise.resolve({ turn: { id: "nt1" } });
      }
      return Promise.resolve(undefined);
    });
    store.models = [DEFAULT_MODEL];
    await sendPrompt("你好");
    expect(mockedInvoke).toHaveBeenCalledWith("goal_set", {
      threadId: "t1",
      objective: "修复登录",
    });
    expect(activeSessionTab()?.goalStatus).toBe("active");
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

describe("buildTurnParams 三面独立映射", () => {
  it("权限/任务模式/模型各自独立派生，互不串扰", () => {
    store.models = [DEFAULT_MODEL];
    const params = buildTurnParams(
      "t1",
      [{ type: "text", text: "hi", text_elements: [] }],
      "cid-1",
      "D:/repo",
      {
        permissionMode: "full-access",
        taskMode: "plan",
        model: "gpt-5-extra",
        effort: "high",
      },
    );
    // 权限面：由 permissionMode 独立映射
    expect(params.approvalPolicy).toBe("never");
    expect(params.sandboxPolicy).toEqual({ type: "dangerFullAccess" });
    // 模型面：params.model 取标签显式值
    expect(params.model).toBe("gpt-5-extra");
    // 计划模式面：mode 由 taskMode 决定，settings.model 取同一标签显式值
    expect(params.collaborationMode).toMatchObject({
      mode: "plan",
      settings: { model: "gpt-5-extra", reasoning_effort: "high" },
    });
  });

  it("只读访问 → never + readOnly 沙箱，无评审", () => {
    store.models = [DEFAULT_MODEL];
    const params = buildTurnParams(
      "t1",
      [{ type: "text", text: "hi", text_elements: [] }],
      "cid-3",
      "D:/repo",
      {
        permissionMode: "read-only",
        taskMode: "default",
        model: "gpt-5",
        effort: null,
      },
    );
    expect(params.approvalPolicy).toBe("never");
    expect(params.sandboxPolicy).toEqual({
      type: "readOnly",
      networkAccess: true,
    });
    expect(params.approvalsReviewer).toBeUndefined();
  });

  it("执行模式 + 标签未选模型时回退默认模型，权限面仍独立", () => {
    store.models = [DEFAULT_MODEL];
    const params = buildTurnParams(
      "t1",
      [{ type: "text", text: "hi", text_elements: [] }],
      "cid-2",
      "D:/repo",
      {
        permissionMode: "help-me-approve",
        taskMode: "default",
        model: null,
        effort: null,
      },
    );
    expect(params.approvalPolicy).toBe("on-request");
    expect(params.model).toBeNull();
    expect(params.collaborationMode).toMatchObject({
      mode: "default",
      settings: { model: "gpt-5" },
    });
  });
});
