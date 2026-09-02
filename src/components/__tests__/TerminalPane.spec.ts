import { afterAll, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
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
    pasteCalls: [] as string[],
    selection: "",
    disposed: 0,
    focusCalls: 0,
    constructorOptions: null as Record<string, unknown> | null,
    options: {} as { theme?: Record<string, unknown> },
  },
}));

vi.mock("@xterm/xterm", () => {
  class TerminalMock {
    constructor(options: Record<string, unknown> = {}) {
      xtermState.constructorOptions = options;
      xtermState.options = { ...options };
    }
    loadAddon() {}
    open() {}
    onData(cb: (d: string) => void) {
      xtermState.onData = cb;
    }
    hasSelection() {
      return !!xtermState.selection;
    }
    getSelection() {
      return xtermState.selection;
    }
    paste(d: string) {
      xtermState.pasteCalls.push(d);
      // 真实 xterm 触发 onData → 前端 terminal_write；mock 同步模拟
      xtermState.onData?.(d);
    }
    write(d: string) {
      xtermState.writeCalls.push(d);
    }
    focus() {
      xtermState.focusCalls++;
    }
    dispose() {
      xtermState.disposed++;
    }
    get options() {
      return xtermState.options;
    }
    set options(v: Record<string, unknown>) {
      xtermState.options = { ...v };
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
    busy: false,
    exited: false,
    exitCode: null,
    ...over,
  }) as unknown as TerminalEditorTab;
}

/** 三套主题的 CSS 变量表（与 src/styles/theme.css 保持一致） */
const THEME_VARS: Record<string, Record<string, string>> = {
  blue: {
    "--bg": "#0e1116",
    "--console-bg-deep": "rgba(12, 16, 22, 0.9)",
    "--console-text": "#c6cdd8",
    "--accent": "#4da6ff",
    "--accent-rgb": "77, 166, 255",
    "--console-cursor-text": "#0c1016",
    "--console-ansi-black": "#0e1116",
    "--console-ansi-red": "#f0565f",
    "--console-ansi-green": "#46d5a8",
    "--console-ansi-yellow": "#e5d6a0",
    "--console-ansi-blue": "#79b8ff",
    "--console-ansi-magenta": "#c792ea",
    "--console-ansi-cyan": "#56b6c2",
    "--console-ansi-white": "#c6cdd8",
    "--console-ansi-bright-black": "#76819a",
    "--console-ansi-bright-red": "#ff7a83",
    "--console-ansi-bright-green": "#7cebc2",
    "--console-ansi-bright-yellow": "#f2e3b0",
    "--console-ansi-bright-blue": "#9ccbff",
    "--console-ansi-bright-magenta": "#e0a7f5",
    "--console-ansi-bright-cyan": "#86d7e0",
    "--console-ansi-bright-white": "#eef2f8",
  },
  dark: {
    "--bg": "#0a0b10",
    "--console-bg-deep": "rgba(12, 13, 20, 0.9)",
    "--console-text": "#b9b8c9",
    "--accent": "#a78bfa",
    "--accent-rgb": "167, 139, 250",
    "--console-cursor-text": "#0c1016",
    "--console-ansi-black": "#0a0b10",
    "--console-ansi-red": "#ff6b74",
    "--console-ansi-green": "#55e0b3",
    "--console-ansi-yellow": "#f0e0a4",
    "--console-ansi-blue": "#8fb3ff",
    "--console-ansi-magenta": "#c792ea",
    "--console-ansi-cyan": "#56b6c2",
    "--console-ansi-white": "#c9c7d6",
    "--console-ansi-bright-black": "#868399",
    "--console-ansi-bright-red": "#ff8d94",
    "--console-ansi-bright-green": "#7cf0c4",
    "--console-ansi-bright-yellow": "#f8eabe",
    "--console-ansi-bright-blue": "#b6c9ff",
    "--console-ansi-bright-magenta": "#e0a7f5",
    "--console-ansi-bright-cyan": "#86d7e0",
    "--console-ansi-bright-white": "#f1effc",
  },
  light: {
    "--bg": "#f4f6fb",
    "--console-bg-deep": "rgba(231, 236, 244, 0.95)",
    "--console-text": "#3e4756",
    "--accent": "#2563eb",
    "--accent-rgb": "37, 99, 235",
    "--console-cursor-text": "#ffffff",
    "--console-ansi-black": "#24292f",
    "--console-ansi-red": "#cf222e",
    "--console-ansi-green": "#116329",
    "--console-ansi-yellow": "#9a6700",
    "--console-ansi-blue": "#0969da",
    "--console-ansi-magenta": "#8250df",
    "--console-ansi-cyan": "#1b7c83",
    "--console-ansi-white": "#57606a",
    "--console-ansi-bright-black": "#6e7781",
    "--console-ansi-bright-red": "#e5534b",
    "--console-ansi-bright-green": "#1a7f37",
    "--console-ansi-bright-yellow": "#9a6700",
    "--console-ansi-bright-blue": "#0969da",
    "--console-ansi-bright-magenta": "#8250df",
    "--console-ansi-bright-cyan": "#1b7c83",
    "--console-ansi-bright-white": "#3e4756",
  },
};

const ANSI_MAP: Array<[string, string]> = [
  ["black", "--console-ansi-black"],
  ["red", "--console-ansi-red"],
  ["green", "--console-ansi-green"],
  ["yellow", "--console-ansi-yellow"],
  ["blue", "--console-ansi-blue"],
  ["magenta", "--console-ansi-magenta"],
  ["cyan", "--console-ansi-cyan"],
  ["white", "--console-ansi-white"],
  ["brightBlack", "--console-ansi-bright-black"],
  ["brightRed", "--console-ansi-bright-red"],
  ["brightGreen", "--console-ansi-bright-green"],
  ["brightYellow", "--console-ansi-bright-yellow"],
  ["brightBlue", "--console-ansi-bright-blue"],
  ["brightMagenta", "--console-ansi-bright-magenta"],
  ["brightCyan", "--console-ansi-bright-cyan"],
  ["brightWhite", "--console-ansi-bright-white"],
];

function applyThemeVars(id: string) {
  for (const [name, value] of Object.entries(THEME_VARS[id])) {
    document.documentElement.style.setProperty(name, value);
  }
  document.documentElement.setAttribute("data-theme", id);
}

function clearThemeVars() {
  for (const vars of Object.values(THEME_VARS)) {
    for (const name of Object.keys(vars)) {
      document.documentElement.style.removeProperty(name);
    }
  }
  document.documentElement.removeAttribute("data-theme");
}

/** 按主题变量表推导 TerminalPane 应同步给 xterm 的完整主题对象 */
function expectedTheme(id: string): Record<string, string> {
  const vars = THEME_VARS[id];
  const theme: Record<string, string> = {
    background: vars["--bg"],
    foreground: vars["--console-text"],
    cursor: vars["--accent"],
    cursorAccent: vars["--console-cursor-text"],
    selectionBackground: `rgba(${vars["--accent-rgb"]}, 0.35)`,
  };
  for (const [key, varName] of ANSI_MAP) {
    theme[key] = vars[varName];
  }
  return theme;
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
    xtermState.pasteCalls.length = 0;
    xtermState.selection = "";
    xtermState.disposed = 0;
    xtermState.focusCalls = 0;
    xtermState.constructorOptions = null;
    xtermState.options = {};
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

  it("回放含提示符标记的缓冲输出：busy 保持 false（初始空闲）", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab();
    bridgeState.buffered.set(tab.id, ["\x1b]133;D\x07PS D:\\repo> "]);
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();
    expect(tab.busy).toBe(false);
    wrapper.unmount();
  });

  it("普通输出不改 busy：无命令执行时不误亮", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab();
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();

    bridgeState.dataHandlers.get(tab.id)!("build output line\r\n");
    await nextTick();
    expect(tab.busy).toBe(false);
    wrapper.unmount();
  });

  it("回车/粘贴换行置 busy=true，收到提示符标记后熄灭", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab();
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();

    xtermState.onData?.("npm run build\r");
    await nextTick();
    expect(tab.busy).toBe(true);

    bridgeState.dataHandlers.get(tab.id)!("\x1b]133;D\x07PS D:\\repo> ");
    await nextTick();
    expect(tab.busy).toBe(false);
    wrapper.unmount();
  });

  it("提示符标记跨两块输出拆分时仍能识别", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab();
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();

    xtermState.onData?.("build\r");
    await nextTick();
    expect(tab.busy).toBe(true);

    bridgeState.dataHandlers.get(tab.id)!("\x1b]133;");
    bridgeState.dataHandlers.get(tab.id)!("D\x07PS D:\\repo> ");
    await nextTick();
    expect(tab.busy).toBe(false);
    wrapper.unmount();
  });

  it("terminal/exit 事件同时熄灭 busy", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab({ busy: true });
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();

    bridgeState.exitHandlers.get(tab.id)!(0);
    await nextTick();
    expect(tab.busy).toBe(false);
    wrapper.unmount();
  });

  it("挂载前进程已退出：缓冲退出码同时熄灭 busy", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab({ busy: true });
    bridgeState.bufferedExit.set(tab.id, 1);
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();
    await nextTick();
    expect(tab.exited).toBe(true);
    expect(tab.busy).toBe(false);
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

  it("挂载时聚焦 xterm（首次打开即可直接输入）", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const wrapper = mount(TerminalPane, {
      props: { tab: makeTab(), active: true },
    });
    await flushPromises();
    expect(xtermState.focusCalls).toBe(1);
    wrapper.unmount();
  });

  it("标签激活（active false→true）后聚焦 xterm", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const wrapper = mount(TerminalPane, {
      props: { tab: makeTab(), active: false },
    });
    await flushPromises();
    expect(xtermState.focusCalls).toBe(0);

    await wrapper.setProps({ active: true });
    await nextTick();
    await flushPromises();
    expect(xtermState.focusCalls).toBe(1);
    wrapper.unmount();
  });

  it("标签切走（active true→false）不再聚焦", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const wrapper = mount(TerminalPane, {
      props: { tab: makeTab(), active: true },
    });
    await flushPromises();
    expect(xtermState.focusCalls).toBe(1);

    await wrapper.setProps({ active: false });
    await nextTick();
    expect(xtermState.focusCalls).toBe(1);
    wrapper.unmount();
  });

  it("已退出终端激活时不聚焦", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    const tab = makeTab({ exited: true, exitCode: 0 });
    const wrapper = mount(TerminalPane, {
      props: { tab, active: false },
    });
    await flushPromises();

    await wrapper.setProps({ active: true });
    await nextTick();
    expect(xtermState.focusCalls).toBe(0);
    wrapper.unmount();
  });

  it.each(["blue", "dark", "light"])(
    "主题 %s：按主题 CSS 变量构造完整 xterm 配色",
    async (id) => {
      applyThemeVars(id);
      mockedInvoke.mockResolvedValue(undefined);
      const wrapper = mount(TerminalPane, { props: { tab: makeTab() } });
      await flushPromises();

      expect(xtermState.constructorOptions?.allowTransparency).toBe(true);
      expect(xtermState.constructorOptions?.theme).toEqual(expectedTheme(id));

      clearThemeVars();
      wrapper.unmount();
    },
  );

  it("data-theme 切换（蓝夜 → 晨光）后同步更新完整 xterm 配色", async () => {
    applyThemeVars("blue");
    mockedInvoke.mockResolvedValue(undefined);
    const wrapper = mount(TerminalPane, { props: { tab: makeTab() } });
    await flushPromises();
    expect(xtermState.options.theme).toEqual(expectedTheme("blue"));

    applyThemeVars("light");
    await nextTick();
    await flushPromises();
    expect(xtermState.options.theme).toEqual(expectedTheme("light"));

    clearThemeVars();
    wrapper.unmount();
  });
});

describe("TerminalPane 复制 / 粘贴", () => {
  function hostOf(wrapper: VueWrapper) {
    return wrapper.find(".terminal-host").element as HTMLElement;
  }

  /** 让 clipboard_read_text 命令返回指定文本（其余命令静默成功） */
  function mockClipboardRead(text: string) {
    mockedInvoke.mockImplementation((cmd) =>
      cmd === "clipboard_read_text"
        ? Promise.resolve(text)
        : Promise.resolve(undefined),
    );
  }

  function rightClick(host: HTMLElement) {
    host.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: 40,
        clientY: 40,
      }),
    );
  }

  /** 派发 paste 事件；传 text 时附带 clipboardData，否则走 clipboard_read_text 回退 */
  function firePaste(host: HTMLElement, text?: string) {
    const ev = new Event("paste", {
      bubbles: true,
      cancelable: true,
    }) as ClipboardEvent;
    if (text !== undefined) {
      Object.defineProperty(ev, "clipboardData", {
        value: { getData: () => text },
        configurable: true,
      });
    }
    host.dispatchEvent(ev);
  }

  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue(undefined);
    xtermState.selection = "";
    xtermState.pasteCalls.length = 0;
  });

  it("有选区时右键弹出「复制」「粘贴」，无选区仅「粘贴」", async () => {
    const wrapper = mount(TerminalPane, { props: { tab: makeTab() } });
    await flushPromises();
    const host = hostOf(wrapper);

    rightClick(host);
    await nextTick();
    expect(wrapper.findAll(".ctx-menu-item").map((b) => b.text())).toEqual([
      "粘贴",
    ]);

    xtermState.selection = "abc";
    rightClick(host);
    await nextTick();
    expect(wrapper.findAll(".ctx-menu-item").map((b) => b.text())).toEqual([
      "复制",
      "粘贴",
    ]);
    wrapper.unmount();
  });

  it("右键「粘贴」经 Rust 读系统剪贴板并写入 ConPTY", async () => {
    const tab = makeTab();
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();
    mockClipboardRead("pasted text");

    rightClick(hostOf(wrapper));
    await nextTick();
    const pasteBtn = wrapper
      .findAll(".ctx-menu-item")
      .find((b) => b.text() === "粘贴")!;
    await pasteBtn.trigger("click");
    await flushPromises();

    expect(mockedInvoke).toHaveBeenCalledWith("clipboard_read_text");
    expect(xtermState.pasteCalls).toEqual(["pasted text"]);
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_write", {
      id: tab.id,
      data: "pasted text",
    });
    wrapper.unmount();
  });

  it("Ctrl+V：clipboardData 有文本时直接写入（不调用 clipboard_read_text）", async () => {
    const tab = makeTab();
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();

    firePaste(hostOf(wrapper), "inline text");
    await flushPromises();

    expect(mockedInvoke).not.toHaveBeenCalledWith("clipboard_read_text");
    expect(xtermState.pasteCalls).toEqual(["inline text"]);
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_write", {
      id: tab.id,
      data: "inline text",
    });
    wrapper.unmount();
  });

  it("Ctrl+V：clipboardData 为空时回退 clipboard_read_text", async () => {
    const tab = makeTab();
    const wrapper = mount(TerminalPane, { props: { tab } });
    await flushPromises();
    mockClipboardRead("fallback text");

    firePaste(hostOf(wrapper));
    await flushPromises();

    expect(mockedInvoke).toHaveBeenCalledWith("clipboard_read_text");
    expect(xtermState.pasteCalls).toEqual(["fallback text"]);
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_write", {
      id: tab.id,
      data: "fallback text",
    });
    wrapper.unmount();
  });
});
