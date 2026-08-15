import { beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { defineComponent } from "vue";
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
});
