import { beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { useChatScroll } from "../useChatScroll";

describe("useChatScroll 回到底部", () => {
  beforeEach(() => {
    vi.stubGlobal("requestAnimationFrame", () => 1);
    vi.stubGlobal("cancelAnimationFrame", () => undefined);
  });

  it("点击回到底部时当刻写入底部，不依赖下一帧", () => {
    const el = document.createElement("div");
    Object.defineProperty(el, "scrollHeight", {
      configurable: true,
      value: 2000,
    });
    el.scrollTop = 300;
    const scroll = useChatScroll(ref(el), {
      isNavAnimating: () => false,
      onTrustedScroll: () => undefined,
      scheduleAnchorSync: () => undefined,
      onSettled: () => undefined,
    });

    scroll.jumpToBottom();

    expect(el.scrollTop).toBe(2000);
    scroll.dispose();
  });
});
