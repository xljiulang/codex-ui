import { beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";

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

import { store } from "../useCodex";

describe("任务栏不确定进度条", () => {
  beforeEach(async () => {
    h.state.shouldThrow = false;
    store.turnActive = false;
    // 先冲刷上一个用例遗留的 watch 回调，再清空 mock，保证基线确定
    await nextTick();
    h.win.setProgressBar.mockClear();
  });

  it("回合进行中显示不确定进度条", async () => {
    store.turnActive = true;
    await nextTick();
    expect(h.win.setProgressBar).toHaveBeenLastCalledWith({
      status: "indeterminate",
    });
  });

  it("回合结束后隐藏进度条", async () => {
    store.turnActive = true;
    await nextTick();
    store.turnActive = false;
    await nextTick();
    expect(h.win.setProgressBar).toHaveBeenLastCalledWith({
      status: "none",
    });
  });

  it("非 Tauri 环境不抛错", async () => {
    h.state.shouldThrow = true;
    store.turnActive = true;
    await nextTick();
    expect(h.win.setProgressBar).not.toHaveBeenCalled();
  });
});
