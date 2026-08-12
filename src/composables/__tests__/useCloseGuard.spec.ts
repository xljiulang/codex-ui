import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { registerCloseGuard } from "../useCloseGuard";
import { settleConfirm, store } from "../useCodex";

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

describe("registerCloseGuard 关闭窗口守卫", () => {
  beforeEach(() => {
    h.state.handler = null;
    h.state.shouldThrow = false;
    h.win.onCloseRequested.mockClear();
    h.win.destroy.mockClear();
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue({});
    store.confirm = null;
    store.turnActive = false;
    store.currentThreadId = null;
    store.currentTurnId = null;
    store.taskMode = "execute";
  });

  it("回合空闲：不阻止关闭、不弹确认", async () => {
    await registerCloseGuard();
    expect(h.state.handler).not.toBeNull();
    const ev = makeCloseEvent();
    await h.state.handler!(ev);
    expect(ev.preventDefault).not.toHaveBeenCalled();
    expect(store.confirm).toBeNull();
    expect(h.win.destroy).not.toHaveBeenCalled();
  });

  it("回合进行中：阻止关闭并弹出确认", async () => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    await registerCloseGuard();
    const ev = makeCloseEvent();
    const pending = h.state.handler!(ev);
    expect(ev.preventDefault).toHaveBeenCalled();
    expect(store.confirm?.title).toBe("关闭应用");
    expect(store.confirm?.message).toContain("关闭将停止当前回合");
    expect(store.confirm?.confirmLabel).toBe("停止并关闭");
    expect(store.confirm?.cancelLabel).toBe("取消");
    settleConfirm(false);
    await pending;
    expect(h.win.destroy).not.toHaveBeenCalled();
  });

  it("点击「停止并关闭」：先停止回合再关闭窗口", async () => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
    await registerCloseGuard();
    const ev = makeCloseEvent();
    const pending = h.state.handler!(ev);
    settleConfirm(true);
    await pending;
    expect(mockedInvoke).toHaveBeenCalledWith(
      "turn_interrupt",
      expect.objectContaining({ threadId: "t1", turnId: "turn-1" }),
    );
    expect(h.win.destroy).toHaveBeenCalledTimes(1);
  });

  it("点击「取消」：不停止回合、不关闭窗口", async () => {
    store.turnActive = true;
    store.currentThreadId = "t1";
    store.currentTurnId = "turn-1";
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
    store.turnActive = true;
    store.currentThreadId = "t1";
    const resolve = vi.fn();
    const existing = {
      title: "切换会话",
      message: "当前对话仍在进行中，切换将停止当前回合。是否继续？",
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
    expect(store.confirm?.message).toContain("切换将停止当前回合");
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
