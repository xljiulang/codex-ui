import { beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, type Component } from "vue";
import { flushPromises } from "@vue/test-utils";

import { useWindowControls } from "../useWindowControls";

/** 在真实组件 setup 中调用 composable，触发其生命周期钩子（onMounted/onBeforeUnmount） */
function withSetup<T>(composable: () => T): { result: T; unmount: () => void } {
  let result!: T;
  const testComp: Component = {
    setup() {
      result = composable();
      return () => null;
    },
  };
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp(testComp);
  app.mount(host);
  return {
    result,
    unmount: () => {
      app.unmount();
      host.remove();
    },
  };
}

const h = vi.hoisted(() => {
  const win = {
    minimize: vi.fn(async () => {}),
    toggleMaximize: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    isMaximized: vi.fn(async () => false),
    onResized: vi.fn(async () => () => {}),
    startDragging: vi.fn(async () => {}),
  };
  return { shouldThrow: false, win };
});

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => {
    if (h.shouldThrow) throw new Error("not in tauri");
    return h.win;
  },
}));

describe("useWindowControls 窗口控制", () => {
  beforeEach(() => {
    h.shouldThrow = false;
    h.win.minimize.mockClear();
    h.win.toggleMaximize.mockClear();
    h.win.close.mockClear();
    h.win.isMaximized.mockClear();
    h.win.startDragging.mockClear();
    h.win.onResized.mockClear();
  });

  it("挂载时按 isMaximized 初始化并注册 resize 监听", async () => {
    h.win.isMaximized.mockResolvedValue(true);
    const { result, unmount } = withSetup(() => useWindowControls());
    await flushPromises();
    expect(result.isMaximized.value).toBe(true);
    expect(h.win.isMaximized).toHaveBeenCalled();
    expect(h.win.onResized).toHaveBeenCalled();
    // 卸载时移除 resize 监听
    unmount();
  });

  it("minimize/close 调用窗口对应 API", async () => {
    const { result } = withSetup(() => useWindowControls());
    await flushPromises();
    result.minimize();
    await flushPromises();
    expect(h.win.minimize).toHaveBeenCalledTimes(1);
    result.close();
    await flushPromises();
    expect(h.win.close).toHaveBeenCalledTimes(1);
  });

  it("toggleMaximize 切换后按 isMaximized 刷新状态", async () => {
    h.win.isMaximized.mockResolvedValue(false);
    let maximized = false;
    h.win.toggleMaximize.mockImplementation(async () => {
      maximized = !maximized;
      h.win.isMaximized.mockResolvedValue(maximized);
    });
    const { result } = withSetup(() => useWindowControls());
    await flushPromises();
    expect(result.isMaximized.value).toBe(false);
    await result.toggleMaximize();
    expect(h.win.toggleMaximize).toHaveBeenCalledTimes(1);
    expect(result.isMaximized.value).toBe(true);
  });

  it("交互元素点击：detail=1 不拖动、detail=2 不最大化", async () => {
    const { result } = withSetup(() => useWindowControls());
    await flushPromises();
    const buttonTarget = {
      button: 0,
      detail: 1,
      target: { closest: () => ({}) },
    };
    result.onTitlebarMouseDown(buttonTarget as unknown as MouseEvent);
    expect(h.win.startDragging).not.toHaveBeenCalled();
    result.onTitlebarMouseDown({
      ...buttonTarget,
      detail: 2,
    } as unknown as MouseEvent);
    expect(h.win.toggleMaximize).not.toHaveBeenCalled();
  });

  it("空白区：detail=1 触发拖动，不触发最大化", async () => {
    const { result } = withSetup(() => useWindowControls());
    await flushPromises();
    const emptyTarget = {
      button: 0,
      detail: 1,
      target: { closest: () => null },
    };
    result.onTitlebarMouseDown(emptyTarget as unknown as MouseEvent);
    expect(h.win.startDragging).toHaveBeenCalledTimes(1);
    expect(h.win.toggleMaximize).not.toHaveBeenCalled();
  });

  it("非左键按下不触发拖动", async () => {
    const { result } = withSetup(() => useWindowControls());
    await flushPromises();
    const rightTarget = {
      button: 2,
      detail: 1,
      target: { closest: () => null },
    };
    result.onTitlebarMouseDown(rightTarget as unknown as MouseEvent);
    expect(h.win.startDragging).not.toHaveBeenCalled();
    expect(h.win.toggleMaximize).not.toHaveBeenCalled();
  });

  it("空白区双击（detail=2）触发最大化，不进入拖动", async () => {
    const { result } = withSetup(() => useWindowControls());
    await flushPromises();
    result.onTitlebarMouseDown({
      button: 0,
      detail: 2,
      target: { closest: () => ({}) },
    } as unknown as MouseEvent);
    expect(h.win.toggleMaximize).not.toHaveBeenCalled();
    result.onTitlebarMouseDown({
      button: 0,
      detail: 2,
      target: { closest: () => null },
    } as unknown as MouseEvent);
    expect(h.win.toggleMaximize).toHaveBeenCalledTimes(1);
    expect(h.win.startDragging).not.toHaveBeenCalled();
  });

  it("非 Tauri 环境：所有操作静默降级且不抛错", async () => {
    h.shouldThrow = true;
    const { result, unmount } = withSetup(() => useWindowControls());
    await flushPromises();
    expect(() => {
      result.minimize();
      void result.toggleMaximize();
      result.close();
      result.onTitlebarMouseDown({
        button: 0,
        detail: 2,
        target: { closest: () => null },
      } as unknown as MouseEvent);
    }).not.toThrow();
    expect(h.win.minimize).not.toHaveBeenCalled();
    expect(h.win.close).not.toHaveBeenCalled();
    unmount();
  });
});
