import { describe, expect, it } from "vitest";
import { ref } from "vue";
import { useTailWindow } from "../useTailWindow";

describe("useTailWindow 增量截断", () => {
  it("初始即窗口化就绪", () => {
    const src = ref("1\n2\n3\n4");
    const { ref: out, truncated } = useTailWindow(src, 2);
    expect(out.value).toBe("3\n4");
    expect(truncated.value).toBe(true);
  });

  it("追加行只留末尾 N 行", () => {
    const src = ref("1\n2\n3\n4");
    const { ref: out } = useTailWindow(src, 2);
    src.value = "1\n2\n3\n4\n5\n6";
    expect(out.value).toBe("5\n6");
  });

  it("无换行追加并入最后一行（窗口不增行）", () => {
    const src = ref("a\nb");
    const { ref: out, truncated } = useTailWindow(src, 1);
    expect(out.value).toBe("b");
    expect(truncated.value).toBe(true);
    src.value = "a\nbc";
    expect(out.value).toBe("bc");
  });

  it("未超限时不截断", () => {
    const src = ref("a\nb");
    const { ref: out, truncated } = useTailWindow(src, 5);
    expect(out.value).toBe("a\nb");
    expect(truncated.value).toBe(false);
  });

  it("超限后 truncated 保持 true", () => {
    const src = ref("1\n2\n3\n4");
    const { ref: out, truncated } = useTailWindow(src, 2);
    src.value = "1\n2\n3\n4\n5\n6\n7\n8";
    expect(truncated.value).toBe(true);
    expect(out.value).toBe("7\n8");
  });

  it("源整体替换（同长度）后重置", () => {
    const src = ref("AAA");
    const reset = ref(0);
    const { ref: out, truncated } = useTailWindow(src, 5, () => reset.value);
    src.value = "BBB";
    reset.value++;
    expect(out.value).toBe("BBB");
    expect(truncated.value).toBe(false);
  });

  it("边界行合并语义与 split 一致", () => {
    const src = ref("1\n2\n3\n4");
    const { ref: out } = useTailWindow(src, 2);
    src.value = "1\n2\n3\n45\n6";
    expect(out.value).toBe("45\n6");
  });
});
