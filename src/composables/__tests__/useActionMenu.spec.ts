import { describe, expect, it, vi } from "vitest";
import { useActionMenu } from "../useActionMenu";

function openMenu(
  m: ReturnType<typeof useActionMenu>,
  label = "打开",
): MouseEvent {
  const e = new MouseEvent("contextmenu", { clientX: 100, clientY: 50 });
  vi.spyOn(e, "preventDefault");
  vi.spyOn(e, "stopPropagation");
  m.openCtx(e, [{ label, action: () => undefined }]);
  return e;
}

describe("useActionMenu 操作右键菜单", () => {
  it("openCtx 记录菜单位置、阻止默认事件并阻止冒泡", () => {
    const m = useActionMenu();
    const e = openMenu(m);
    expect(e.preventDefault).toHaveBeenCalled();
    expect(e.stopPropagation).toHaveBeenCalled();
    expect(m.ctxMenu.value).not.toBeNull();
    expect(m.ctxMenu.value?.items[0].label).toBe("打开");
  });

  it("窗口点击关闭菜单", () => {
    const m = useActionMenu();
    openMenu(m);
    expect(m.ctxMenu.value).not.toBeNull();
    m.onWindowClick();
    expect(m.ctxMenu.value).toBeNull();
  });

  it("Escape 关闭菜单并返回 true；无菜单时返回 false", () => {
    const m = useActionMenu();
    expect(
      m.onKeydown(new KeyboardEvent("keydown", { key: "Escape" })),
    ).toBe(false);
    openMenu(m);
    expect(
      m.onKeydown(new KeyboardEvent("keydown", { key: "Escape" })),
    ).toBe(true);
    expect(m.ctxMenu.value).toBeNull();
  });

  it("滚动仅在被声明的作用域内关闭菜单", () => {
    const m = useActionMenu({ scrollScope: ".panel" });
    openMenu(m);
    const outside = document.createElement("div");
    outside.className = "other";
    m.onWindowScroll({ target: outside } as unknown as Event);
    expect(m.ctxMenu.value).not.toBeNull();
    const inside = document.createElement("div");
    inside.className = "panel";
    m.onWindowScroll({ target: inside } as unknown as Event);
    expect(m.ctxMenu.value).toBeNull();
  });
});
