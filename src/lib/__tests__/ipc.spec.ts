import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { assetUrl, call, subscribe } from "../ipc";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const mockedInvoke = vi.mocked(invoke);
const mockedListen = vi.mocked(listen);
const mockedConvert = vi.mocked(convertFileSrc);

function setTauri(v: boolean) {
  const w = window as unknown as { __TAURI_INTERNALS__?: unknown };
  if (v) w.__TAURI_INTERNALS__ = {};
  else delete w.__TAURI_INTERNALS__;
}

describe("ipc 传输层", () => {
  beforeEach(() => {
    setTauri(true);
    sessionStorage.clear();
    mockedInvoke.mockReset();
    mockedListen.mockReset();
    mockedListen.mockResolvedValue(() => {});
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    setTauri(true);
  });

  it("Tauri 模式 call 委托 invoke（含参数）", async () => {
    mockedInvoke.mockResolvedValue("ok");
    await expect(call("server_status", { a: 1 })).resolves.toBe("ok");
    expect(mockedInvoke).toHaveBeenCalledWith("server_status", { a: 1 });
  });

  it("Tauri 模式无参数调用只传命令名", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    await call("workspace_dir");
    expect(mockedInvoke).toHaveBeenCalledWith("workspace_dir");
    expect(mockedInvoke.mock.calls[0]).toHaveLength(1);
  });

  it("Tauri 模式 subscribe 委托 listen", async () => {
    const handler = vi.fn();
    const un = await subscribe<{ x: number }>("evt", handler);
    expect(mockedListen).toHaveBeenCalledWith("evt", expect.any(Function));
    un();
  });

  it("Tauri 模式 assetUrl 走 asset 协议", () => {
    mockedConvert.mockReturnValue("asset://D:/a.png");
    expect(assetUrl("D:/a.png")).toBe("asset://D:/a.png");
  });

  it("远程模式 call 走 /rpc：snake 键 + Bearer 令牌", async () => {
    setTauri(false);
    sessionStorage.setItem("codex_ui_remote_token", "tok123");
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ ok: true, data: { n: 1 } }),
    });
    vi.stubGlobal("fetch", fetchMock);
    const res = await call("session_fs_copy", {
      root: "R",
      src: "S",
      destDir: "D",
    });
    expect(res).toEqual({ n: 1 });
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/rpc");
    expect(init.method).toBe("POST");
    expect(init.headers.Authorization).toBe("Bearer tok123");
    expect(JSON.parse(init.body)).toEqual({
      cmd: "session_fs_copy",
      args: { root: "R", src: "S", dest_dir: "D" },
    });
  });

  it("远程模式 call 错误形状抛错", async () => {
    setTauri(false);
    sessionStorage.setItem("codex_ui_remote_token", "tok123");
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({ ok: false, error: "boom" }),
      }),
    );
    await expect(call("x")).rejects.toThrow("boom");
  });

  it("远程模式 assetUrl 带令牌", () => {
    setTauri(false);
    sessionStorage.setItem("codex_ui_remote_token", "tok123");
    expect(assetUrl("D:/a.png")).toBe(
      "/asset?path=D%3A%2Fa.png&token=tok123",
    );
  });

  it("远程模式 subscribe 经 WebSocket 分发并按事件过滤", async () => {
    setTauri(false);
    sessionStorage.setItem("codex_ui_remote_token", "tok123");
    class FakeWS {
      static instances: FakeWS[] = [];
      readyState = 1;
      onmessage: ((e: { data: string }) => void) | null = null;
      onclose: (() => void) | null = null;
      constructor(public url: string) {
        FakeWS.instances.push(this);
      }
      send() {}
      close() {}
    }
    vi.stubGlobal("WebSocket", FakeWS);

    const handler = vi.fn();
    const un = await subscribe<number>("turn/completed", handler);
    const inst = FakeWS.instances[0];
    expect(inst.url).toContain("/events?token=tok123");

    inst.onmessage?.({
      data: JSON.stringify({ event: "turn/completed", payload: 7 }),
    });
    inst.onmessage?.({ data: JSON.stringify({ event: "other", payload: 9 }) });
    expect(handler).toHaveBeenCalledTimes(1);
    expect(handler).toHaveBeenCalledWith({ payload: 7 });

    un();
    inst.onmessage?.({
      data: JSON.stringify({ event: "turn/completed", payload: 8 }),
    });
    expect(handler).toHaveBeenCalledTimes(1);
  });
});
