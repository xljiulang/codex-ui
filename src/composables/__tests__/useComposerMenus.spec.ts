import { beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";

vi.mock("../useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../useCodex")>();
  return { ...mod };
});

import { store } from "../useCodex";
import {
  useComposerMenus,
  type ComposerMention,
} from "../useComposerMenus";

function resetMenus() {
  store.permOpen = false;
  store.taskOpen = false;
  store.modelOpen = false;
}

describe("useComposerMenus", () => {
  beforeEach(() => {
    resetMenus();
  });

  it("toggleMenu 打开一个时关闭另外两个，再点当前按钮关闭", () => {
    const m = useComposerMenus({ mention: ref<ComposerMention>(null) });
    m.toggleMenu("perm");
    expect(store.permOpen).toBe(true);
    m.toggleMenu("task");
    expect(store.permOpen).toBe(false);
    expect(store.taskOpen).toBe(true);
    m.toggleMenu("task");
    expect(store.taskOpen).toBe(false);
  });

  it("closeMenus 清空全部菜单与 mention", () => {
    const mention = ref<ComposerMention>({ kind: "@", token: "a", start: 0 });
    const m = useComposerMenus({ mention });
    store.permOpen = true;
    store.modelOpen = true;
    m.closeMenus();
    expect(store.permOpen).toBe(false);
    expect(store.taskOpen).toBe(false);
    expect(store.modelOpen).toBe(false);
    expect(mention.value).toBeNull();
  });

  it("Escape 关闭菜单", () => {
    const m = useComposerMenus({ mention: ref<ComposerMention>(null) });
    store.permOpen = true;
    m.onKeydownGlobal({ key: "Escape" } as KeyboardEvent);
    expect(store.permOpen).toBe(false);
  });

  it("mousedown：非 Element 或外部关闭，弹层内部保持打开", () => {
    const m = useComposerMenus({ mention: ref<ComposerMention>(null) });

    // 非 Element 目标（window 等）→ 关闭
    store.permOpen = true;
    m.onWindowMousedown({ target: window } as unknown as MouseEvent);
    expect(store.permOpen).toBe(false);

    // 弹层内部 → 保持打开
    store.permOpen = true;
    const inside = document.createElement("div");
    inside.className = "popup-menu";
    m.onWindowMousedown({ target: inside } as unknown as MouseEvent);
    expect(store.permOpen).toBe(true);

    // 外部 Element → 关闭
    const outside = document.createElement("div");
    m.onWindowMousedown({ target: outside } as unknown as MouseEvent);
    expect(store.permOpen).toBe(false);
  });
});
