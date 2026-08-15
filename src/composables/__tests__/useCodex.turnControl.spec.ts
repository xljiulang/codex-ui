import { newEmptyChat, openThread, sendPrompt } from "../useCodex/actions";
import { __resetSessionTabsForTest } from "../useCodex/sessionState";
import { store } from "../useCodex/store";
import { clearGoal, setGoal } from "../useCodex/turnControl";
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
