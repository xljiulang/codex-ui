import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  loadHistoryDetailPage,
  loadThreadHistory,
  mirrorHistoryItem,
} from "../useCodex/history";
import { store } from "../useCodex/store";
import { makeSessionTab, resetUseCodexState } from "./useCodexTestHarness";

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

function user(id: string) {
  return {
    id,
    type: "userMessage",
    content: [{ type: "text", text: id, text_elements: [] }],
  };
}

function agent(id: string) {
  return { id, type: "agentMessage", text: id };
}

function command(id: string) {
  return {
    id,
    type: "commandExecution",
    command: id,
    status: "completed",
  };
}

function setupHistoryMock() {
  mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
    if (cmd !== "codex_rpc") return Promise.resolve(undefined);
    const { method, params } = args as {
      method: string;
      params: Record<string, unknown>;
    };
    if (method === "thread/turns/list") {
      if (params.itemsView !== "summary") throw new Error("应读取 summary");
      if (params.cursor === "summary-2") {
        return Promise.resolve({
          data: [
            {
              id: "turn-1",
              status: "completed",
              startedAt: 10,
              items: [user("u1"), agent("a1")],
            },
          ],
          nextCursor: null,
        });
      }
      return Promise.resolve({
        data: [
          {
            id: "turn-3",
            status: "completed",
            startedAt: 30,
            items: [user("u3"), agent("a3")],
          },
          {
            id: "turn-2",
            status: "completed",
            startedAt: 20,
            items: [user("u2"), agent("a2")],
          },
        ],
        nextCursor: "summary-2",
      });
    }
    if (method === "thread/items/list") {
      const turnId = String(params.turnId);
      return Promise.resolve({
        data: [
          { turnId, item: agent(`a${turnId.slice(-1)}`) },
          {
            turnId,
            item: command(`c${turnId.slice(-1)}-latest`),
          },
          {
            turnId,
            item: command(`c${turnId.slice(-1)}-older`),
          },
        ],
        nextCursor: turnId === "turn-2" ? "turn-2-older" : null,
        backwardsCursor: null,
      });
    }
    throw new Error(`unexpected method ${method}`);
  });
}

beforeEach(() => {
  resetUseCodexState(mockedInvoke, mockedListen);
  mockedInvoke.mockReset();
  store.itemsByThread = {};
});

describe("会话历史窗口加载", () => {
  it("先读全量摘要骨架，再仅为最新两个回合各取一页 100 条明细", async () => {
    setupHistoryMock();
    const tab = makeSessionTab("s1", "thread", { history: undefined });

    expect(await loadThreadHistory(tab, "thread")).toBe(true);

    expect(tab.history?.turns.map((turn) => turn.id)).toEqual([
      "turn-1",
      "turn-2",
      "turn-3",
    ]);
    expect(tab.history?.turns.map((turn) => turn.detailLoaded)).toEqual([
      false,
      true,
      true,
    ]);
    expect(store.itemsByThread.thread.map((item) => item.id)).toEqual([
      "u1",
      "a1",
      "u2",
      "c2-older",
      "c2-latest",
      "a2",
      "u3",
      "c3-older",
      "c3-latest",
      "a3",
    ]);
    const calls = mockedInvoke.mock.calls.map(([, args]) => args);
    expect(calls).toContainEqual({
      method: "thread/items/list",
      params: {
        threadId: "thread",
        turnId: "turn-3",
        cursor: null,
        limit: 100,
        sortDirection: "desc",
        itemView: "full",
      },
    });
    expect(calls).toContainEqual({
      method: "thread/items/list",
      params: {
        threadId: "thread",
        turnId: "turn-2",
        cursor: null,
        limit: 100,
        sortDirection: "desc",
        itemView: "full",
      },
    });
    expect(calls).not.toContainEqual(
      expect.objectContaining({
        params: expect.objectContaining({ itemsView: "full" }),
      }),
    );
  });

  it("点击未加载旧回合只请求该回合并原地合成明细", async () => {
    setupHistoryMock();
    const tab = makeSessionTab("s1", "thread", { history: undefined });
    await loadThreadHistory(tab, "thread");
    mockedInvoke.mockClear();

    expect(await loadHistoryDetailPage(tab, "thread", "turn-1")).toBe(true);

    expect(mockedInvoke).toHaveBeenCalledTimes(1);
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/items/list",
      params: {
        threadId: "thread",
        turnId: "turn-1",
        cursor: null,
        limit: 100,
        sortDirection: "desc",
        itemView: "full",
      },
    });
    expect(tab.history?.turns[0].detailLoaded).toBe(true);
    expect(
      store.itemsByThread.thread.slice(0, 6).map((item) => item.id),
    ).toEqual(["u1", "c1-older", "c1-latest", "a1", "u2", "c2-older"]);
  });

  it("实时条目写入历史明细源，后续补载不会覆盖流式更新", async () => {
    setupHistoryMock();
    const tab = makeSessionTab("s1", "thread", { history: undefined });
    await loadThreadHistory(tab, "thread");

    mirrorHistoryItem(tab, "turn-3", {
      id: "a3",
      type: "agentMessage",
      text: "实时最终答复",
    });
    await loadHistoryDetailPage(tab, "thread", "turn-1");

    expect(
      tab.history?.turns[2].detailItems.find((item) => item.id === "a3"),
    ).toMatchObject({ text: "实时最终答复" });
    expect(
      store.itemsByThread.thread.find((item) => item.id === "a3"),
    ).toMatchObject({ text: "实时最终答复" });
  });

  it("同一时刻只允许一个历史明细请求", async () => {
    setupHistoryMock();
    const tab = makeSessionTab("s1", "thread", { history: undefined });
    await loadThreadHistory(tab, "thread");
    let finish: ((value: unknown) => void) | undefined;
    mockedInvoke.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );

    const first = loadHistoryDetailPage(tab, "thread", "turn-1");
    const second = await loadHistoryDetailPage(tab, "thread", "turn-2");

    expect(second).toBe(false);
    finish?.({
      data: [{ turnId: "turn-1", item: command("c1-older") }],
      nextCursor: null,
    });
    expect(await first).toBe(true);
  });
});
