import { beforeEach, describe, expect, it, vi } from "vitest";

const { listeners, listen } = vi.hoisted(() => {
  const listeners = new Map<string, (e: { payload: unknown }) => void>();
  return {
    listeners,
    listen: vi
      .fn()
      .mockImplementation(
        (event: string, cb: (e: { payload: unknown }) => void) => {
          listeners.set(event, cb);
          return Promise.resolve(() => listeners.delete(event));
        },
      ),
  };
});

vi.mock("@tauri-apps/api/event", () => ({ listen }));

import {
  __resetTerminalEventsForTest,
  attachTerminal,
  ensureTerminalListeners,
  releaseTerminal,
} from "../useTerminalEvents";

function emitOutput(id: string, data: string): void {
  listeners.get("terminal/output")!({ payload: { id, data } });
}

function emitExit(id: string, exitCode: number): void {
  listeners.get("terminal/exit")!({ payload: { id, exitCode } });
}

describe("useTerminalEvents 事件桥", () => {
  beforeEach(() => {
    listeners.clear();
    listen.mockClear();
    __resetTerminalEventsForTest();
  });

  it("监听只注册一次：output 与 exit 各一个全局监听", async () => {
    await ensureTerminalListeners();
    await ensureTerminalListeners();
    expect(listeners.has("terminal/output")).toBe(true);
    expect(listeners.has("terminal/exit")).toBe(true);
    expect(listen).toHaveBeenCalledTimes(2);
  });

  it("挂载前到达的输出被缓冲，attach 后 flush 取走并清空", async () => {
    await ensureTerminalListeners();
    emitOutput("t1", "a");
    emitOutput("t1", "b");
    const h = attachTerminal("t1");
    expect(h.flush()).toEqual(["a", "b"]);
    expect(h.flush()).toEqual([]);
    expect(h.exitCode).toBeNull();
  });

  it("挂载后事件经 onData 实时分发且不重复进缓冲", async () => {
    await ensureTerminalListeners();
    const h = attachTerminal("t1");
    const received: string[] = [];
    h.onData((d) => received.push(d));
    emitOutput("t1", "live");
    expect(received).toEqual(["live"]);
    expect(h.flush()).toEqual([]);
  });

  it("退出码：attach 前到达由 exitCode 读取，attach 后经 onExit 分发", async () => {
    await ensureTerminalListeners();
    emitExit("t1", 3);
    expect(attachTerminal("t1").exitCode).toBe(3);

    const h2 = attachTerminal("t2");
    const codes: number[] = [];
    h2.onExit((c) => codes.push(c));
    emitExit("t2", 7);
    expect(codes).toEqual([7]);
  });

  it("detach 停止实时分发，后续输出回到缓冲", async () => {
    await ensureTerminalListeners();
    const h = attachTerminal("t1");
    const received: string[] = [];
    h.onData((d) => received.push(d));
    h.detach();
    emitOutput("t1", "after-detach");
    expect(received).toEqual([]);
    expect(attachTerminal("t1").flush()).toEqual(["after-detach"]);
  });

  it("release 清空该 id 的缓冲与退出码", async () => {
    await ensureTerminalListeners();
    emitOutput("t1", "x");
    emitExit("t1", 1);
    releaseTerminal("t1");
    const h = attachTerminal("t1");
    expect(h.flush()).toEqual([]);
    expect(h.exitCode).toBeNull();
  });

  it("多 id 缓冲与实时分发互不串扰", async () => {
    await ensureTerminalListeners();
    emitOutput("t1", "for-t1");
    const h1 = attachTerminal("t1");
    const h2 = attachTerminal("t2");
    expect(h1.flush()).toEqual(["for-t1"]);
    expect(h2.flush()).toEqual([]);

    const got: string[] = [];
    h2.onData((d) => got.push(d));
    emitOutput("t2", "live2");
    emitOutput("t1", "live1");
    expect(got).toEqual(["live2"]);
  });
});
