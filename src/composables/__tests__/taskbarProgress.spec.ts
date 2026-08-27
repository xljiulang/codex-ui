import { beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick, reactive } from "vue";

const h = vi.hoisted(() => {
  const win = {
    setProgressBar: vi.fn(async () => {}),
  };
  const state = { shouldThrow: false };
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
  ProgressBarStatus: {
    None: "none",
    Normal: "normal",
    Indeterminate: "indeterminate",
    Paused: "paused",
    Error: "error",
  },
}));

import { activeTabId, tabs } from "../useEditorTabs";
import { __resetSessionTabsForTest } from "../useCodex/sessionState";
import type { SessionTab } from "../useCodex/types";
import {
  fireListen,
  makeSessionTab,
  resetUseCodexState,
} from "./useCodexTestHarness";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { wireEvents } from "../useCodex/events";

const mockedInvoke = vi.mocked(invoke);
const mockedListen = vi.mocked(listen);
const sessionTab = () => tabs[0] as SessionTab;

describe("任务栏不确定进度条", () => {
  beforeEach(async () => {
    h.state.shouldThrow = false;
    resetUseCodexState(mockedInvoke, mockedListen);
    // 活动会话标签（reactive，供 activeSessionTab()?.turnActive 响应式触发）
    tabs.push(reactive(makeSessionTab("s1", "t1", { turnActive: false })));
    activeTabId.value = "s1";
    // 先冲刷上一个用例遗留的 watch 回调，再清空 mock，保证基线确定
    await nextTick();
    h.win.setProgressBar.mockClear();
  });

  it("回合进行中显示不确定进度条", async () => {
    sessionTab().turnActive = true;
    await nextTick();
    expect(h.win.setProgressBar).toHaveBeenLastCalledWith({
      status: "indeterminate",
    });
  });

  it("回合结束后隐藏进度条", async () => {
    sessionTab().turnActive = true;
    await nextTick();
    sessionTab().turnActive = false;
    await nextTick();
    expect(h.win.setProgressBar).toHaveBeenLastCalledWith({
      status: "none",
    });
  });

  it("非 Tauri 环境不抛错", async () => {
    h.state.shouldThrow = true;
    sessionTab().turnActive = true;
    await nextTick();
    expect(h.win.setProgressBar).not.toHaveBeenCalled();
  });

  it("taskbar-progress-refresh 事件：按当前工作态重新应用进度条", async () => {
    sessionTab().turnActive = true;
    await nextTick();
    expect(h.win.setProgressBar).toHaveBeenLastCalledWith({
      status: "indeterminate",
    });

    // 注册 wireEvents 以监听 taskbar-progress-refresh（模拟窗口从托盘恢复的通知）
    await wireEvents();
    h.win.setProgressBar.mockClear();
    fireListen("taskbar-progress-refresh", undefined);
    await nextTick();
    expect(h.win.setProgressBar).toHaveBeenLastCalledWith({
      status: "indeterminate",
    });

    // 无工作态时刷新 → none
    sessionTab().turnActive = false;
    await nextTick();
    h.win.setProgressBar.mockClear();
    fireListen("taskbar-progress-refresh", undefined);
    await nextTick();
    expect(h.win.setProgressBar).toHaveBeenLastCalledWith({ status: "none" });
  });
});
