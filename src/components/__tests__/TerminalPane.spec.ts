import { afterAll, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick, reactive } from "vue";

const { bridgeState, xtermState } = vi.hoisted(() => ({
  bridgeState: {
    attached: [] as string[],
    detached: [] as string[],
    dataHandlers: new Map<string, (data: string) => void>(),
    exitHandlers: new Map<string, (exitCode: number) => void>(),
    buffered: new Map<string, string[]>(),
    bufferedExit: new Map<string, number>(),
  },
  xtermState: {
    onData: null as null | ((d: string) => void),
    writeCalls: [] as string[],
    disposed: 0,
  },
}));

vi.mock("@xterm/xterm", () => {
  class TerminalMock {
    loadAddon() {}
    open() {}
    onData(cb: (d: string) => void) {
      xtermState.onData = cb;
    }
    write(d: string) {
      xtermState.writeCalls.push(d);
    }
    focus() {}
    dispose() {
      xtermState.disposed++;
    }
  }
  return { Terminal: TerminalMock };
});

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit() {}
    proposeDimensions() {
      return { cols: 100, rows: 30 };
    }
  },
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../composables/useTerminalEvents", () => ({
  ensureTerminalListeners: vi.fn(() => Promise.resolve()),
  attachTerminal: vi.fn((id: string) => {
    bridgeState.attached.push(id);
    return {
      flush: () => {
        const buf = bridgeState.buffered.get(id) ?? [];
        bridgeState.buffered.delete(id);
        return buf;
      },
      get exitCode() {
        return bridgeState.bufferedExit.get(id) ?? null;
      },
      onData: (cb: (data: string) => void) => {
        bridgeState.dataHandlers.set(id, cb);
      },
      onExit: (cb: (exitCode: number) => void) => {
        bridgeState.exitHandlers.set(id, cb);
      },
      detach: () => {
        bridgeState.detached.push(id);
        bridgeState.dataHandlers.delete(id);
        bridgeState.exitHandlers.delete(id);
      },
    };
  }),
  releaseTerminal: vi.fn(),
}));

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("ResizeObserver", ResizeObserverStub);

import { invoke } from "@tauri-apps/api/core";
import TerminalPane from "../TerminalPane.vue";
import type { TerminalEditorTab } from "../../composables/useEditorTabs";

const mockedInvoke = vi.mocked(invoke);

function makeTab(over: Partial<TerminalEditorTab> = {}): TerminalEditorTab {
  return reactive({
    kind: "terminal",
    id: "terminal:1:1",
    cwd: "D:\\repo",
    title: "repo",
    loading: false,
    error: "",
    exited: false,
    exitCode: null,
    ...over,
  }) as unknown as TerminalEditorTab;
}

describe("TerminalPane", () => {
  beforeEach(() => {
    bridgeState.attached.length = 0;
    bridgeState.detached.length = 0;
    bridgeState.dataHandlers.clear();
    bridgeState.exitHandlers.clear();
    bridgeState.buffered.clear();
    bridgeState.bufferedExit.clear();
    xtermState.onData = null;
    xtermState.writeCalls.length = 0;
    xtermState.disposed = 0;
    mockedInvoke.mockReset();
  });

  afterAll(() => {
    vi.unstubAllGlobals();
  });

  it("挂载后创建 xterm、经事件桥订阅并同步初始尺寸", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab();
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();
    expect(bridgeState.attached).toContain(tab.id);
    expect(bridgeState.dataHandlers.has(tab.id)).toBe(true);
    expect(bridgeState.exitHandlers.has(tab.id)).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_resize", {
      id: tab.id,
      cols: 100,
      rows: 30,
    });
    wrapper.unmount();
  });

  it("挂载即回放事件桥缓冲输出（首个终端的 DSR 启动输出不丢失）", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab();
    bridgeState.buffered.set(tab.id, ["\x1b[6nPS D:\\repo>"]);
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();
    expect(xtermState.writeCalls).toEqual(["\x1b[6nPS D:\\repo>"]);
    wrapper.unmount();
  });

  it("挂载前进程已退出：缓冲退出码直接显示已退出覆盖层", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab();
    bridgeState.bufferedExit.set(tab.id, 1);
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();
    await nextTick();
    expect(tab.exited).toBe(true);
    expect(tab.exitCode).toBe(1);
    expect(wrapper.find(".terminal-overlay").text()).toContain("进程已退出");
    wrapper.unmount();
  });

  it("xterm onData 输入转发为 terminal_write", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab();
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();
    xtermState.onData?.("ls\r");
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_write", {
      id: tab.id,
      data: "ls\r",
    });
    wrapper.unmount();
  });

  it("事件桥实时输出写入 xterm（订阅按本标签 id 建立）", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab();
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();

    expect(bridgeState.dataHandlers.has("terminal:other")).toBe(false);
    bridgeState.dataHandlers.get(tab.id)!("hi");
    expect(xtermState.writeCalls).toEqual(["hi"]);
    bridgeState.dataHandlers.get(tab.id)!(" again");
    expect(xtermState.writeCalls).toEqual(["hi", " again"]);
    wrapper.unmount();
  });

  it("terminal/exit 事件标记标签已退出并显示覆盖层", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab();
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();

    bridgeState.exitHandlers.get(tab.id)!(0);
    await nextTick();
    expect(tab.exited).toBe(true);
    expect(tab.exitCode).toBe(0);
    expect(wrapper.find(".terminal-overlay").text()).toContain("进程已退出");
    wrapper.unmount();
  });

  it("spawn 完成（loading 置 false）后再次 fit 并同步尺寸", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab({ loading: true });
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_resize", {
      id: tab.id,
      cols: 100,
      rows: 30,
    });
    const resizeCalls = mockedInvoke.mock.calls.filter(
      (c) => c[0] === "terminal_resize",
    ).length;
    expect(resizeCalls).toBe(1);

    tab.loading = false;
    await nextTick();
    expect(mockedInvoke).toHaveBeenCalledTimes(resizeCalls + 1);
    expect(mockedInvoke).toHaveBeenLastCalledWith("terminal_resize", {
      id: tab.id,
      cols: 100,
      rows: 30,
    });
    wrapper.unmount();
  });

  it("卸载时 detach 事件桥并销毁 xterm", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const wrapper = mount(TerminalPane, { props: { tab: makeTab() } });
    await flushPromises();
    wrapper.unmount();
    expect(bridgeState.detached).toHaveLength(1);
    expect(xtermState.disposed).toBe(1);
  });
});
