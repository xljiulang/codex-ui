import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { registerCloseGuard } from "../useCloseGuard";
import { interrupt } from "../useCodex";
import { tabs } from "../useEditorTabs";
import type { SessionTab } from "../useCodex";
import type { TerminalEditorTab } from "../useEditorTabs";

interface CloseEventLike {
  preventDefault: ReturnType<typeof vi.fn>;
}

const h = vi.hoisted(() => {
  const state: {
    closeHandler: ((event: CloseEventLike) => void | Promise<void>) | null;
    exitHandler: (() => void | Promise<void>) | null;
    shouldThrow: boolean;
    tabs: Array<Record<string, unknown>>;
  } = {
    closeHandler: null,
    exitHandler: null,
    shouldThrow: false,
    tabs: [],
  };
  return { state };
});

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => {
    if (h.state.shouldThrow) throw new Error("not in tauri");
    return {
      onCloseRequested: async (cb: (event: CloseEventLike) => void | Promise<void>) => {
        h.state.closeHandler = cb;
        return () => {};
      },
    };
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, cb: (payload: unknown) => void | Promise<void>) => {
    if (name === "app-exit-requested") h.state.exitHandler = cb as () => void | Promise<void>;
    return () => {};
  },
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

vi.mock("../useCodex", () => ({ interrupt: vi.fn() }));

vi.mock("../useEditorTabs", () => ({ tabs: h.state.tabs }));

vi.mock("../../lib/tabs", () => ({
  isTabWorking: vi.fn((t: { working?: boolean }) => t.working === true),
  TabKind: { Chat: "chat", Terminal: "terminal" },
}));

const mockedInvoke = vi.mocked(invoke);
const mockedInterrupt = vi.mocked(interrupt);

function sessionTab(
  over: Partial<SessionTab> & { working?: boolean } = {},
): SessionTab & { working: boolean } {
  return {
    id: "s1",
    kind: "chat",
    title: "会话",
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
    working: false,
    ...over,
  };
}

function terminalTab(id = "term-1"): TerminalEditorTab & { working: boolean } {
  return {
    kind: "terminal",
    id,
    workspace: "D:/repo",
    title: "cmd",
    icon: "terminal",
    loading: false,
    error: "",
    busy: false,
    exited: false,
    exitCode: null,
    working: true,
  };
}

describe("registerCloseGuard 退出守卫", () => {
  beforeEach(() => {
    h.state.closeHandler = null;
    h.state.exitHandler = null;
    h.state.shouldThrow = false;
    h.state.tabs.length = 0;
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue({});
    mockedInterrupt.mockReset();
    mockedInterrupt.mockResolvedValue(undefined);
  });

  it("窗口关闭：统一 preventDefault，不触发 app_exit", async () => {
    await registerCloseGuard();
    expect(h.state.closeHandler).not.toBeNull();
    const ev = { preventDefault: vi.fn() };
    await h.state.closeHandler!(ev);
    expect(ev.preventDefault).toHaveBeenCalledTimes(1);
    expect(mockedInvoke).not.toHaveBeenCalledWith("app_exit", expect.anything());
  });

  it("app-exit-requested：中断工作会话、终止工作终端，最后调用 app_exit", async () => {
    tabs.push(
      sessionTab({ id: "s1", threadId: "t1", currentTurnId: "turn-1", working: true }),
    );
    tabs.push(
      sessionTab({ id: "s2", threadId: "t2", currentTurnId: "turn-2", working: false }),
    );
    tabs.push(terminalTab("term-1"));

    await registerCloseGuard();
    expect(h.state.exitHandler).not.toBeNull();
    await h.state.exitHandler!();

    expect(mockedInterrupt).toHaveBeenCalledWith("t1", "turn-1");
    // 非工作会话不中断
    expect(mockedInterrupt).not.toHaveBeenCalledWith("t2", expect.anything());
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_kill", { id: "term-1" });

    const calls = mockedInvoke.mock.calls;
    expect(calls[calls.length - 1][0]).toBe("app_exit");
  });

  it("app-exit-requested：中断抛错仍调用 app_exit（finally 兜底）", async () => {
    tabs.push(
      sessionTab({ id: "s1", threadId: "t1", currentTurnId: "turn-1", working: true }),
    );
    mockedInterrupt.mockRejectedValueOnce(new Error("中断失败"));

    await registerCloseGuard();
    await h.state.exitHandler!();

    expect(mockedInvoke).toHaveBeenCalledWith("app_exit");
  });

  it("非 Tauri 环境：注册返回 no-op 且不抛错", async () => {
    h.state.shouldThrow = true;
    const unlisten = await registerCloseGuard();
    expect(typeof unlisten).toBe("function");
    expect(() => unlisten()).not.toThrow();
  });
});
