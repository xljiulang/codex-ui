import { afterAll, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick, reactive } from "vue";

const { listeners, xtermState } = vi.hoisted(() => ({
  listeners: new Map<string, (e: { payload: unknown }) => void>(),
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
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockImplementation(
    (event: string, cb: (e: { payload: unknown }) => void) => {
      listeners.set(event, cb);
      return Promise.resolve(() => listeners.delete(event));
    },
  ),
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
    listeners.clear();
    xtermState.onData = null;
    xtermState.writeCalls.length = 0;
    xtermState.disposed = 0;
    mockedInvoke.mockReset();
  });

  afterAll(() => {
    vi.unstubAllGlobals();
  });

  it("挂载后创建 xterm、注册事件监听并同步初始尺寸", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab();
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();
    expect(listeners.has("terminal/output")).toBe(true);
    expect(listeners.has("terminal/exit")).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_resize", {
      id: tab.id,
      cols: 100,
      rows: 30,
    });
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

  it("terminal/output 只写入匹配 id 的 xterm，忽略其它终端输出", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab();
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();

    listeners.get("terminal/output")!({
      payload: { id: "terminal:other", data: "noise" },
    });
    expect(xtermState.writeCalls).toHaveLength(0);

    listeners.get("terminal/output")!({
      payload: { id: tab.id, data: "hi" },
    });
    expect(xtermState.writeCalls).toEqual(["hi"]);
    wrapper.unmount();
  });

  it("terminal/exit 事件标记标签已退出并显示覆盖层", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab();
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();

    listeners.get("terminal/exit")!({
      payload: { id: tab.id, exitCode: 0 },
    });
    await nextTick();
    expect(tab.exited).toBe(true);
    expect(tab.exitCode).toBe(0);
    expect(wrapper.find(".terminal-overlay").text()).toContain("进程已退出");
    wrapper.unmount();
  });

  it("卸载时取消监听并销毁 xterm", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const wrapper = mount(TerminalPane, { props: { tab: makeTab() } });
    await flushPromises();
    wrapper.unmount();
    expect(xtermState.disposed).toBe(1);
    expect(listeners.size).toBe(0);
  });
});
