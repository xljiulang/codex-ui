import { init } from "../useCodex/boot";
import { disposeEvents } from "../useCodex/events";
import { __resetSessionTabsForTest } from "../useCodex/sessionState";
import { store } from "../useCodex/store";
import { activeTabId } from "../useEditorTabs";
import { capturedListeners, mockListenCapture, resetUseCodexState, tabs } from "./useCodexTestHarness";
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

describe("启动加载态 booting 状态", () => {
  beforeEach(() => {
    disposeEvents(); // 重置 wired，避免 init() 内的 wireEvents 与其它用例互相干扰
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    __resetSessionTabsForTest();
    store.booting = true;
  });

  it("初始为 true，init() 完成后置为 false", async () => {
    expect(store.booting).toBe(true);
    await init();
    expect(store.booting).toBe(false);
    // 允许 0 个会话标签：启动不自动创建
    expect(tabs).toHaveLength(0);
    expect(activeTabId.value).toBe("");
  });

  it("init() 抛错时也关闭加载态", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "server_status") {
        return Promise.reject(new Error("后端不可用"));
      }
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    expect(store.booting).toBe(true);
    await expect(init()).rejects.toThrow("后端不可用");
    expect(store.booting).toBe(false);
  });
});
