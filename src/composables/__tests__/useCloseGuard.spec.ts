import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { registerCloseGuard } from "../useCloseGuard";
import { settleConfirm, store } from "../useCodex";
import {
  __resetEditorTabsForTest,
  tabs,
  type TerminalEditorTab,
} from "../useEditorTabs";
import type { SessionTab } from "../useCodex";

interface CloseEventLike {
  preventDefault: ReturnType<typeof vi.fn>;
  isPreventDefault: ReturnType<typeof vi.fn>;
}

const h = vi.hoisted(() => {
  const state: {
    handler: ((event: CloseEventLike) => void | Promise<void>) | null;
    shouldThrow: boolean;
  } = {
    handler: null,
    shouldThrow: false,
  };
  const win = {
    onCloseRequested: vi.fn(
      async (cb: (event: CloseEventLike) => void | Promise<void>) => {
        state.handler = cb;
        return () => {};
      },
    ),
    destroy: vi.fn(async () => {}),
  };
  return { state, win };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => {
    if (h.state.shouldThrow) throw new Error("not in tauri");
    return h.win;
  },
}));

const mockedInvoke = vi.mocked(invoke);

function makeCloseEvent(): CloseEventLike {
  let prevented = false;
  return {
    preventDefault: vi.fn(() => {
      prevented = true;
    }),
    isPreventDefault: vi.fn(() => prevented),
  };
}

/** 构造工作/空闲会话标签 */
function sessionTab(
  over: Partial<SessionTab> = {},
): SessionTab {
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
    plan: null,
    loading: false,
    newChatWorkspace: null,
    interactions: [],
    ...over,
  };
}

function busyTerminal(id = "term-1"): TerminalEditorTab {
  return {
    kind: "terminal",
    id,
    workspace: "D:/repo",
    title: "cmd",
    icon: "terminal",
    loading: false,
    error: "",
    busy: true,
    exited: false,
    exitCode: null,
  };
}

describe("registerCloseGuard 关闭窗口守卫", () => {
  beforeEach(() => {
    h.state.handler = null;
    h.state.shouldThrow = false;
    h.win.onCloseRequested.mockClear();
    h.win.destroy.mockClear();
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue({});
    store.confirm = null;
    tabs.splice(0, tabs.length);
    __resetEditorTabsForTest();
  });

  it("无工作标签且无脏文件：不阻止关闭、不弹确认", async () => {
    await registerCloseGuard();
    expect(h.state.handler).not.toBeNull();
    const ev = makeCloseEvent();
    await h.state.handler!(ev);
    expect(ev.preventDefault).not.toHaveBeenCalled();
    expect(store.confirm).toBeNull();
    expect(h.win.destroy).not.toHaveBeenCalled();
  });

  it("有工作会话：阻止关闭并弹出确认（提示会话数量）", async () => {
    tabs.push(
      sessionTab({ turnActive: true, currentTurnId: "turn-1" }),
    );
    await registerCloseGuard();
    const ev = makeCloseEvent();
    const pending = h.state.handler!(ev);
    expect(ev.preventDefault).toHaveBeenCalled();
    expect(store.confirm?.title).toBe("关闭应用");
    expect(store.confirm?.message).toContain("1 个会话");
    expect(store.confirm?.confirmLabel).toBe("停止并关闭");
    expect(store.confirm?.cancelLabel).toBe("取消");
    settleConfirm(false);
    await pending;
    expect(h.win.destroy).not.toHaveBeenCalled();
  });

  it("点击「停止并关闭」：停止所有工作会话（含后台标签）再关闭窗口", async () => {
    tabs.push(
      sessionTab({ turnActive: true, currentTurnId: "turn-1" }),
    );
    tabs.push(
      sessionTab({
        id: "s2",
        threadId: "t2",
        turnActive: true,
        currentTurnId: "turn-2",
      }),
    );
    await registerCloseGuard();
    const ev = makeCloseEvent();
    const pending = h.state.handler!(ev);
    settleConfirm(true);
    await pending;
    expect(mockedInvoke).toHaveBeenCalledWith(
      "turn_interrupt",
      expect.objectContaining({ threadId: "t1", turnId: "turn-1" }),
    );
    expect(mockedInvoke).toHaveBeenCalledWith(
      "turn_interrupt",
      expect.objectContaining({ threadId: "t2", turnId: "turn-2" }),
    );
    expect(h.win.destroy).toHaveBeenCalledTimes(1);
  });

  it("目标激活续跑（无进行中回合）也视为工作：确认后清目标", async () => {
    tabs.push(
      sessionTab({ goalText: "目标", goalStatus: "active" }),
    );
    await registerCloseGuard();
    const ev = makeCloseEvent();
    const pending = h.state.handler!(ev);
    expect(store.confirm?.message).toContain("1 个会话");
    settleConfirm(true);
    await pending;
    expect(mockedInvoke).toHaveBeenCalledWith(
      "goal_clear",
      expect.objectContaining({ threadId: "t1" }),
    );
    expect(h.win.destroy).toHaveBeenCalledTimes(1);
  });

  it("有运行中终端：阻止关闭，确认后 terminal_kill 再关闭", async () => {
    tabs.push(busyTerminal());
    await registerCloseGuard();
    const ev = makeCloseEvent();
    const pending = h.state.handler!(ev);
    expect(ev.preventDefault).toHaveBeenCalled();
    expect(store.confirm?.message).toContain("1 个终端");
    settleConfirm(true);
    await pending;
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_kill", {
      id: "term-1",
    });
    expect(h.win.destroy).toHaveBeenCalledTimes(1);
  });

  it("点击「取消」：不停止、不关闭窗口", async () => {
    tabs.push(
      sessionTab({ turnActive: true, currentTurnId: "turn-1" }),
    );
    await registerCloseGuard();
    const ev = makeCloseEvent();
    const pending = h.state.handler!(ev);
    settleConfirm(false);
    await pending;
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "turn_interrupt",
      expect.anything(),
    );
    expect(h.win.destroy).not.toHaveBeenCalled();
  });

  it("已有确认框（如切换会话）时：仅阻止关闭，不覆盖原确认", async () => {
    tabs.push(
      sessionTab({ turnActive: true, currentTurnId: "turn-1" }),
    );
    const resolve = vi.fn();
    const existing = {
      title: "切换会话",
      message: "当前会话仍在进行中，切换将停止当前回合。是否继续？",
      confirmLabel: "停止并切换",
      cancelLabel: "取消",
      resolve,
    };
    store.confirm = existing;
    await registerCloseGuard();
    const ev = makeCloseEvent();
    await h.state.handler!(ev);
    expect(ev.preventDefault).toHaveBeenCalled();
    expect(store.confirm).not.toBeNull();
    expect(store.confirm?.title).toBe("切换会话");
    expect(resolve).not.toHaveBeenCalled();
    expect(h.win.destroy).not.toHaveBeenCalled();
  });

  it("非 Tauri 环境：注册返回 no-op 且不抛错", async () => {
    h.state.shouldThrow = true;
    const unlisten = await registerCloseGuard();
    expect(typeof unlisten).toBe("function");
    expect(() => unlisten()).not.toThrow();
  });
});
