import { beforeEach, describe, expect, it } from "vitest";
import { ref } from "vue";
import {
  useComposerMenus,
  type ComposerMention,
} from "../useComposerMenus";

describe("useComposerMenus", () => {
  beforeEach(() => {
    document.body.innerHTML = ""; // 清掉跨用例残留的 DOM
  });

  it("toggleMenu 打开一个时关闭另外两个，再点当前按钮关闭", () => {
    const m = useComposerMenus({ mention: ref<ComposerMention>(null) });
    m.toggleMenu("perm");
    expect(m.permOpen.value).toBe(true);
    m.toggleMenu("collab");
    expect(m.permOpen.value).toBe(false);
    expect(m.collabOpen.value).toBe(true);
    m.toggleMenu("collab");
    expect(m.collabOpen.value).toBe(false);
  });

  it("closeMenus 清空全部菜单与 mention", () => {
    const mention = ref<ComposerMention>({ kind: "@", token: "a", start: 0 });
    const m = useComposerMenus({ mention });
    m.permOpen.value = true;
    m.modelOpen.value = true;
    m.closeMenus();
    expect(m.permOpen.value).toBe(false);
    expect(m.collabOpen.value).toBe(false);
    expect(m.modelOpen.value).toBe(false);
    expect(mention.value).toBeNull();
  });

  it("不同实例互不串扰：一个实例的开关不影响另一个", () => {
    const a = useComposerMenus({ mention: ref<ComposerMention>(null) });
    const b = useComposerMenus({ mention: ref<ComposerMention>(null) });
    a.toggleMenu("perm");
    expect(a.permOpen.value).toBe(true);
    expect(b.permOpen.value).toBe(false);
    a.closeMenus();
    expect(b.permOpen.value).toBe(false);
    b.toggleMenu("model");
    expect(b.modelOpen.value).toBe(true);
    expect(a.modelOpen.value).toBe(false);
  });

  it("Escape 关闭菜单", () => {
    const m = useComposerMenus({ mention: ref<ComposerMention>(null) });
    m.permOpen.value = true;
    m.onKeydownGlobal({ key: "Escape" } as KeyboardEvent);
    expect(m.permOpen.value).toBe(false);
  });

  it("mousedown：非 Element 或外部关闭，弹层内部保持打开", () => {
    const m = useComposerMenus({ mention: ref<ComposerMention>(null) });

    // 非 Element 目标（window 等）→ 关闭
    m.permOpen.value = true;
    m.onWindowMousedown({ target: window } as unknown as MouseEvent);
    expect(m.permOpen.value).toBe(false);

    // 弹层内部 → 保持打开
    m.permOpen.value = true;
    const inside = document.createElement("div");
    inside.className = "popup-menu";
    m.onWindowMousedown({ target: inside } as unknown as MouseEvent);
    expect(m.permOpen.value).toBe(true);

    // 外部 Element → 关闭
    const outside = document.createElement("div");
    m.onWindowMousedown({ target: outside } as unknown as MouseEvent);
    expect(m.permOpen.value).toBe(false);
  });
});
