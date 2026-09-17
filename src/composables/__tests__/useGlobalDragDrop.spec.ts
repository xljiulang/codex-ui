import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";

// Mock must be before imports
const mockOnDragDropEvent = vi.fn(
  async (
    _handler: (event: {
      payload: {
        type: "enter" | "over" | "leave" | "drop";
        paths?: string[];
        position?: { x: number; y: number };
      };
    }) => void,
  ) => {
    return () => {};
  },
);

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: mockOnDragDropEvent,
  }),
}));

vi.mock("../useSessionFs", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../useSessionFs")>();
  return { ...mod, openPathInApp: vi.fn().mockResolvedValue(true) };
});

vi.mock("../useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../useCodex")>();
  return { ...mod, setToast: vi.fn() };
});

import { setToast } from "../useCodex";
import { openPathInApp } from "../useSessionFs";
import { registerDropTarget, unregisterDropTarget, type DropTarget } from "../dropTargets";
import { useGlobalDragDrop } from "../useGlobalDragDrop";

const mockedOpenPathInApp = vi.mocked(openPathInApp);
const mockedToast = vi.mocked(setToast);

/** 竖直流失的注册目标，由 afterEach 清理 */
let cleanupTargets: DropTarget[] = [];

function fakeEl(rect: {
  left: number;
  top: number;
  width: number;
  height: number;
}): HTMLElement {
  return {
    getBoundingClientRect: () => ({
      left: rect.left,
      top: rect.top,
      right: rect.left + rect.width,
      bottom: rect.top + rect.height,
      width: rect.width,
      height: rect.height,
      x: rect.left,
      y: rect.top,
      toJSON: () => ({}),
    }),
  } as HTMLElement;
}

function registerTargetRect(rect: {
  left: number;
  top: number;
  width: number;
  height: number;
}) {
  const setDragging = vi.fn();
  const onDropPaths = vi.fn();
  const target: DropTarget = {
    el: fakeEl(rect),
    setDragging,
    onDropPaths,
  };
  cleanupTargets.push(target);
  registerDropTarget(target);
  return { target, setDragging, onDropPaths };
}

/** 抓取全局监听者注册的处理器 */
function handler() {
  return mockOnDragDropEvent.mock.calls[0][0] as (
    event: {
      payload: {
        type: "enter" | "over" | "leave" | "drop";
        paths?: string[];
        position?: { x: number; y: number };
      };
    },
  ) => void;
}

function dropEvent(
  paths: string[],
  position?: { x: number; y: number },
) {
  const payload: {
    type: "drop";
    paths: string[];
    position?: { x: number; y: number };
  } = { type: "drop", paths };
  if (position) payload.position = position;
  return { payload };
}

describe("useGlobalDragDrop 单一全局拖拽监听", () => {
  beforeEach(() => {
    cleanupTargets = [];
    mockedOpenPathInApp.mockReset();
    mockedOpenPathInApp.mockResolvedValue(true);
    mockedToast.mockClear();
    mockOnDragDropEvent.mockClear();
  });

  afterEach(() => {
    for (const t of cleanupTargets) unregisterDropTarget(t);
    cleanupTargets = [];
  });

  it("drop 未命中输入区 → openPathInApp 打开文件", async () => {
    // 模拟 AppHeader 位置（无命中目标）
    const { setup } = useGlobalDragDrop();
    await setup();
    await flushPromises();

    handler()(dropEvent(["D:\\repo\\a.ts", "D:\\repo\\b.cs"], { x: 10, y: 10 }));
    await flushPromises();

    expect(mockedOpenPathInApp).toHaveBeenCalledTimes(2);
    expect(mockedOpenPathInApp).toHaveBeenCalledWith("D:\\repo\\a.ts");
    expect(mockedOpenPathInApp).toHaveBeenCalledWith("D:\\repo\\b.cs");
  });

  it("drop 命中输入区 → 只走附件 onDropPaths，不打开文件", async () => {
    const { onDropPaths, setDragging } = registerTargetRect({
      left: 0,
      top: 400,
      width: 800,
      height: 200,
    });
    const { setup } = useGlobalDragDrop();
    await setup();
    await flushPromises();

    // devicePixelRatio 默认 1：CSS 坐标即物理坐标
    // 先悬停进输入区（点亮高亮），再 drop（熄灭 + 添加附件）
    handler()({
      payload: { type: "enter", paths: [], position: { x: 400, y: 500 } },
    });
    expect(setDragging).toHaveBeenLastCalledWith(true);
    setDragging.mockClear();
    handler()(
      dropEvent(["D:\\repo\\a.ts"], { x: 400, y: 500 }),
    );
    await flushPromises();

    expect(onDropPaths).toHaveBeenCalledWith(["D:\\repo\\a.ts"]);
    expect(mockedOpenPathInApp).not.toHaveBeenCalled();
    // drop 时熄灭高亮
    expect(setDragging).toHaveBeenLastCalledWith(false);
  });

  it("enter/over 命中输入区 → dragging=true 且推送高亮；leave 熄灭", async () => {
    const { setDragging } = registerTargetRect({
      left: 0,
      top: 400,
      width: 800,
      height: 200,
    });
    const { dragging, setup } = useGlobalDragDrop();
    await setup();
    await flushPromises();

    expect(dragging.value).toBe(false);

    // enter 在输入区内
    handler()({
      payload: { type: "enter", paths: [], position: { x: 400, y: 500 } },
    });
    expect(dragging.value).toBe(true);
    expect(setDragging).toHaveBeenCalledWith(true);

    // over 移到输入区外 → 熄灭高亮但 dragging 仍为 true
    setDragging.mockClear();
    handler()({
      payload: { type: "over", position: { x: 50, y: 50 } },
    });
    expect(dragging.value).toBe(true);
    expect(setDragging).toHaveBeenCalledWith(false);

    // leave
    handler()({ payload: { type: "leave" } });
    expect(dragging.value).toBe(false);
  });

  it("openPathInApp 返回 false 时 toast 提示", async () => {
    mockedOpenPathInApp.mockResolvedValueOnce(false);
    const { setup } = useGlobalDragDrop();
    await setup();
    await flushPromises();

    handler()(
      dropEvent(["D:\\repo\\unsupported.bin"], { x: 10, y: 10 }),
    );
    await flushPromises();

    expect(mockedToast).toHaveBeenCalledWith(
      expect.stringContaining("无法打开文件"),
    );
  });

  it("openPathInApp 抛错时 toast 错误信息", async () => {
    mockedOpenPathInApp.mockRejectedValueOnce(new Error("permission denied"));
    const { setup } = useGlobalDragDrop();
    await setup();
    await flushPromises();

    handler()(dropEvent(["D:\\repo\\a.ts"], { x: 10, y: 10 }));
    await flushPromises();

    expect(mockedToast).toHaveBeenCalledWith(
      expect.stringContaining("permission denied"),
    );
  });
});
