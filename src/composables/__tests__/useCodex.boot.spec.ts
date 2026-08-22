import { init } from "../useCodex/boot";
import { disposeEvents } from "../useCodex/events";
import { __resetSessionTabsForTest } from "../useCodex/sessionState";
import { store } from "../useCodex/store";
import { activeTabId } from "../useEditorTabs";
import { capturedListeners, mockListenCapture, resetUseCodexState, tabs } from "./useCodexTestHarness";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn(),
}));

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
const mockedGetVersion = vi.mocked(getVersion);

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

  it("test_hook_enabled=true（E2E 启动）时暴露 __CODEX_UI_TEST__", async () => {
    delete (window as unknown as Record<string, unknown>).__CODEX_UI_TEST__;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "test_hook_enabled") return Promise.resolve(true);
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    await init();
    const hook = (
      window as unknown as { __CODEX_UI_TEST__?: { newSession?: unknown } }
    ).__CODEX_UI_TEST__;
    expect(typeof hook?.newSession).toBe("function");
  });

  it("test_hook_enabled=false（正常启动）时不暴露 __CODEX_UI_TEST__", async () => {
    delete (window as unknown as Record<string, unknown>).__CODEX_UI_TEST__;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "test_hook_enabled") return Promise.resolve(false);
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });
    await init();
    expect(
      (window as unknown as Record<string, unknown>).__CODEX_UI_TEST__,
    ).toBeUndefined();
  });
});

describe("主窗口标题显示版本号", () => {
  beforeEach(() => {
    disposeEvents(); // 重置 wired，避免 init() 内的 wireEvents 与其它用例互相干扰
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedGetVersion.mockReset();
    mockWin.setTitle.mockClear();
  });

  it("Tauri 环境（getVersion 可用）时标题为 Codex UI v<版本>", async () => {
    mockedGetVersion.mockResolvedValue("0.1.1");
    await init();
    expect(mockWin.setTitle).toHaveBeenCalledWith("Codex UI v0.1.1");
  });

  it("getVersion 不可用（非 Tauri）时回退无版本标题 Codex UI", async () => {
    mockedGetVersion.mockRejectedValue(new Error("not in tauri"));
    await init();
    expect(mockWin.setTitle).toHaveBeenCalledWith("Codex UI");
  });
});

describe("启动版本警告（仅低于 0.149.0）", () => {
  beforeEach(() => {
    disposeEvents(); // 重置 wired，避免 init() 内的 wireEvents 与其它用例互相干扰
    for (const k of Object.keys(capturedListeners)) delete capturedListeners[k];
    mockListenCapture();
    mockedInvoke.mockReset();
    __resetSessionTabsForTest();
    store.booting = true;
    store.toast = "";
  });

  it("versionTooOld=true：一次性提示未适配并带版本号", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "server_status") {
        return Promise.resolve({
          connected: true,
          startupWorkspace: "D:/repo",
          codexPath: null,
          codexVersion: "codex-cli 0.148.2",
          versionTooOld: true,
          logs: [],
        });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });

    await init();

    await vi.waitFor(() => {
      expect(store.toast).toContain("codex-cli 0.148.2");
      expect(store.toast).toContain("低于 0.149.0");
    }, { timeout: 3000, interval: 20 });
  });

  it("versionTooOld=false（0.149.x 或更高）：不提示", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "server_status") {
        return Promise.resolve({
          connected: true,
          startupWorkspace: "D:/repo",
          codexPath: null,
          codexVersion: "codex-cli 0.149.0",
          versionTooOld: false,
          logs: [],
        });
      }
      if (cmd === "thread_list") {
        return Promise.resolve({ data: [], nextCursor: null });
      }
      return Promise.resolve(undefined);
    });

    await init();
    await new Promise((r) => setTimeout(r, 500));
    expect(store.toast).toBe("");
  });
});
