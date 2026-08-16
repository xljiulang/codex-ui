import { beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { defineComponent, ref } from "vue";
import type { FsEntry } from "../../lib/sessionFs";
import { useResourceDragDrop } from "../useResourceDragDrop";

function entry(path: string, name = "a.txt"): FsEntry {
  return {
    name,
    path,
    relPath: path,
    isDir: false,
    size: 1,
    modifiedAtMs: 0,
    createdAtMs: 0,
    childCount: null,
  };
}

function pointer(clientX: number, clientY: number): PointerEvent {
  return new MouseEvent("pointermove", {
    clientX,
    clientY,
  }) as unknown as PointerEvent;
}

/** 构造带可控矩形区域的拖拽边界元素（happy-dom 的 getBoundingClientRect 默认全 0） */
function makeBoundary(): HTMLElement {
  const el = document.createElement("div");
  document.body.appendChild(el);
  Object.defineProperty(el, "getBoundingClientRect", {
    configurable: true,
    value: () => ({
      left: 0,
      top: 0,
      right: 100,
      bottom: 100,
      width: 100,
      height: 100,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }),
  });
  return el;
}

describe("useResourceDragDrop", () => {
  beforeEach(() => {
    document.body.classList.remove("resource-dragging");
  });

  it("按下未超阈值不激活；超阈值激活并显示幽灵；松手无目标不移动", async () => {
    const moveEntry = vi.fn().mockResolvedValue(true);
    const Wrapper = defineComponent({
      setup() {
        const d = useResourceDragDrop(moveEntry);
        return { d };
      },
      template: "<div />",
    });
    const wrapper = mount(Wrapper);
    const d = wrapper.vm.d as ReturnType<typeof useResourceDragDrop>;

    d.onRowPointerDown(
      entry("D:\\repo\\a.txt"),
      { button: 0, clientX: 10, clientY: 10 } as PointerEvent,
    );
    expect(d.isDragging.value).toBe(true);

    // 移动 3px（阈值 5 内）→ 不激活
    window.dispatchEvent(pointer(13, 10));
    expect(d.dragActive.value).toBe(false);

    // 超过阈值 → 激活 + 幽灵 + body 类
    window.dispatchEvent(pointer(20, 10));
    expect(d.dragActive.value).toBe(true);
    expect(d.dragGhost.value?.name).toBe("a.txt");
    expect(document.body.classList.contains("resource-dragging")).toBe(true);

    // 松手（无命中目标）→ 不移动、清理状态
    window.dispatchEvent(
      new MouseEvent("pointerup") as unknown as PointerEvent,
    );
    expect(moveEntry).not.toHaveBeenCalled();
    expect(d.isDragging.value).toBe(false);
    expect(document.body.classList.contains("resource-dragging")).toBe(false);
    wrapper.unmount();
  });

  it("Escape 路径 cancelDrag 立即复位", () => {
    const moveEntry = vi.fn().mockResolvedValue(true);
    const Wrapper = defineComponent({
      setup() {
        const d = useResourceDragDrop(moveEntry);
        return { d };
      },
      template: "<div />",
    });
    const wrapper = mount(Wrapper);
    const d = wrapper.vm.d as ReturnType<typeof useResourceDragDrop>;

    d.onRowPointerDown(entry("D:\\repo\\a.txt"), {
      button: 0,
      clientX: 0,
      clientY: 0,
    } as PointerEvent);
    window.dispatchEvent(pointer(100, 100));
    expect(d.dragActive.value).toBe(true);
    d.cancelDrag();
    expect(d.isDragging.value).toBe(false);
    expect(d.dragGhost.value).toBeNull();
    expect(d.dragOverPath.value).toBeNull();
    wrapper.unmount();
  });

  it("指针离开文件区隐藏幽灵与高亮、回到区内恢复，区外松手不移动", () => {
    const moveEntry = vi.fn().mockResolvedValue(true);
    const list = makeBoundary();
    const listRef = ref<HTMLElement | null>(list);
    const Wrapper = defineComponent({
      setup() {
        const d = useResourceDragDrop(moveEntry, listRef);
        return { d };
      },
      template: "<div />",
    });
    const wrapper = mount(Wrapper);
    const d = wrapper.vm.d as ReturnType<typeof useResourceDragDrop>;

    d.onRowPointerDown(
      entry("D:\\repo\\a.txt"),
      { button: 0, clientX: 10, clientY: 10 } as PointerEvent,
    );
    // 区内激活：幽灵显示
    window.dispatchEvent(pointer(30, 30));
    expect(d.dragActive.value).toBe(true);
    expect(d.dragGhost.value?.name).toBe("a.txt");

    // 移出边界：幽灵与高亮清空，拖拽保持
    window.dispatchEvent(pointer(150, 150));
    expect(d.dragGhost.value).toBeNull();
    expect(d.dragOverPath.value).toBeNull();
    expect(d.dragActive.value).toBe(true);

    // 移回边界：幽灵恢复
    window.dispatchEvent(pointer(40, 40));
    expect(d.dragGhost.value?.name).toBe("a.txt");

    // 区外松手：不移动、状态清理
    window.dispatchEvent(pointer(150, 150));
    window.dispatchEvent(new MouseEvent("pointerup") as unknown as PointerEvent);
    expect(moveEntry).not.toHaveBeenCalled();
    expect(d.isDragging.value).toBe(false);
    expect(document.body.classList.contains("resource-dragging")).toBe(false);
    wrapper.unmount();
    list.remove();
  });
});
