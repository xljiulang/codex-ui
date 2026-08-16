import { describe, expect, it } from "vitest";
import { useInputHistory } from "../useInputHistory";

describe("useInputHistory", () => {
  it("prev 按倒序返回历史，push 后回到新输入态", () => {
    const h = useInputHistory();
    h.push("a");
    h.push("b");
    expect(h.prev()).toBe("b");
    expect(h.prev()).toBe("a");
    expect(h.prev()).toBe("a"); // 已到最旧，停留
    h.push("c");
    expect(h.prev()).toBe("c");
  });

  it("next 前进到末尾返回空串（清空），未浏览返回 null", () => {
    const h = useInputHistory();
    h.push("a");
    h.push("b");
    expect(h.next()).toBe(null); // 未开始浏览
    expect(h.prev()).toBe("b");
    expect(h.prev()).toBe("a");
    expect(h.next()).toBe("b");
    expect(h.next()).toBe("");
    expect(h.next()).toBe(null); // 已回到新输入态
  });

  it("无历史时 prev 返回 null", () => {
    const h = useInputHistory();
    expect(h.prev()).toBe(null);
    expect(h.next()).toBe(null);
  });

  it("超过上限时淘汰最旧记录", () => {
    const h = useInputHistory(2);
    h.push("a");
    h.push("b");
    h.push("c");
    expect(h.prev()).toBe("c");
    expect(h.prev()).toBe("b");
    expect(h.prev()).toBe("b");
  });
});
