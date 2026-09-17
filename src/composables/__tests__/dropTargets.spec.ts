import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  findDropTarget,
  registerDropTarget,
  unregisterDropTarget,
  updateDropTarget,
  type DropTarget,
} from "../dropTargets";

/** 模块级登记的所有目标：beforeEach 统一注销，隔离用例 */
let registered: DropTarget[] = [];

/** 构造矩形可控的假元素（getBoundingClientRect 返回给定矩形） */
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

function makeTarget(el: HTMLElement): {
  target: DropTarget;
  setDragging: ReturnType<typeof vi.fn>;
  onDropPaths: ReturnType<typeof vi.fn>;
} {
  const setDragging = vi.fn();
  const onDropPaths = vi.fn();
  const target = { el, setDragging, onDropPaths };
  registered.push(target);
  return {
    target,
    setDragging,
    onDropPaths,
  };
}

describe("dropTargets 全局拖拽接收者注册表", () => {
  beforeEach(() => {
    for (const t of registered) unregisterDropTarget(t);
    registered = [];
  });

  it("findDropTarget 命中包含坐标的可见目标", () => {
    const el = fakeEl({ left: 100, top: 200, width: 400, height: 300 });
    const { target } = makeTarget(el);
    registerDropTarget(target);

    expect(findDropTarget(150, 250)).toBe(target);
    // 边界点也算命中
    expect(findDropTarget(100, 200)).toBe(target);
    expect(findDropTarget(500, 500)).toBe(target);
    // 矩形外不命中
    expect(findDropTarget(99, 250)).toBeNull();
    expect(findDropTarget(150, 501)).toBeNull();
  });

  it("隐藏目标（宽高为 0）被跳过", () => {
    const visible = fakeEl({ left: 0, top: 0, width: 100, height: 100 });
    const hidden = fakeEl({
      left: 0,
      top: 0,
      width: 0,
      height: 0,
    });
    const { target: visibleTarget } = makeTarget(visible);
    const { target: hiddenTarget } = makeTarget(hidden);
    registerDropTarget(visibleTarget);
    registerDropTarget(hiddenTarget);

    // 两个矩形重叠位置：只命中可见目标
    expect(findDropTarget(10, 10)).toBe(visibleTarget);
  });

  it("多个命中取最后注册的目标", () => {
    const el1 = fakeEl({ left: 0, top: 0, width: 500, height: 500 });
    const el2 = fakeEl({ left: 0, top: 0, width: 500, height: 500 });
    const { target: t1 } = makeTarget(el1);
    const { target: t2 } = makeTarget(el2);
    registerDropTarget(t1);
    registerDropTarget(t2);

    expect(findDropTarget(10, 10)).toBe(t2);
  });

  it("unregisterDropTarget 后不再命中", () => {
    const el = fakeEl({ left: 0, top: 0, width: 100, height: 100 });
    const { target } = makeTarget(el);
    registerDropTarget(target);
    expect(findDropTarget(10, 10)).toBe(target);

    unregisterDropTarget(target);
    expect(findDropTarget(10, 10)).toBeNull();
  });

  it("updateDropTarget 切换高亮：先复位旧目标再点亮新目标", () => {
    const { target: t1, setDragging: s1 } = makeTarget(
      fakeEl({ left: 0, top: 0, width: 10, height: 10 }),
    );
    const { target: t2, setDragging: s2 } = makeTarget(
      fakeEl({ left: 0, top: 0, width: 10, height: 10 }),
    );

    updateDropTarget(t1);
    expect(s1).toHaveBeenCalledWith(true);

    updateDropTarget(t2);
    expect(s1).toHaveBeenLastCalledWith(false);
    expect(s2).toHaveBeenCalledWith(true);

    updateDropTarget(null);
    expect(s2).toHaveBeenLastCalledWith(false);
  });

  it("注销高亮目标时自动熄灭", () => {
    const { target, setDragging } = makeTarget(
      fakeEl({ left: 0, top: 0, width: 10, height: 10 }),
    );
    updateDropTarget(target);
    expect(setDragging).toHaveBeenLastCalledWith(true);

    unregisterDropTarget(target);
    expect(setDragging).toHaveBeenLastCalledWith(false);
  });
});
