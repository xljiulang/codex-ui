import { resolveSessionWorkspace, workspace } from "../useCodex/items";
import { __resetSessionTabsForTest } from "../useCodex/sessionState";
import { store } from "../useCodex/store";
import { refreshThreads, searchThreads } from "../useCodex/threads";
import { resetUseCodexState } from "./useCodexTestHarness";
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
