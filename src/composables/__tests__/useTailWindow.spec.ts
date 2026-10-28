import { afterEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { useTailWindow } from "../useTailWindow";

describe("useTailWindow 增量截断", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("初始即窗口化就绪", () => {
    const src = ref("1\n2\n3\n4");
    const { ref: out, truncated } = useTailWindow(src, 2);
    expect(out.value).toBe("3\n4");
    expect(truncated.value).toBe(true);
  });

  it("追加行只留末尾 N 行", () => {
    const src = ref("1\n2\n3\n4");
    const { ref: out, flush } = useTailWindow(src, 2);
    src.value = "1\n2\n3\n4\n5\n6";
    flush();
    expect(out.value).toBe("5\n6");
  });

  it("无换行追加并入最后一行（窗口不增行）", () => {
    const src = ref("a\nb");
    const { ref: out, truncated, flush } = useTailWindow(src, 1);
    expect(out.value).toBe("b");
    expect(truncated.value).toBe(true);
    src.value = "a\nbc";
    flush();
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
    const { ref: out, truncated, flush } = useTailWindow(src, 2);
    src.value = "1\n2\n3\n4\n5\n6\n7\n8";
    flush();
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
    const { ref: out, flush } = useTailWindow(src, 2);
    src.value = "1\n2\n3\n45\n6";
    flush();
    expect(out.value).toBe("45\n6");
  });

  it("超长单行按 UTF-8 字节保留末尾且不切断中文", () => {
    const maxBytes = 512 * 1024;
    const src = ref("中".repeat(200_000));
    const {
      ref: out,
      truncated,
      truncation,
    } = useTailWindow(src, {
      maxLines: 5000,
      maxBytes,
    });

    expect(new TextEncoder().encode(out.value).length).toBeLessThanOrEqual(
      maxBytes,
    );
    expect(out.value.endsWith("中")).toBe(true);
    expect(out.value.includes("\uFFFD")).toBe(false);
    expect(truncated.value).toBe(true);
    expect(truncation.value).toBe("bytes");
  });

  it("分块追加超长单行时窗口始终保持字节上限", () => {
    const maxBytes = 512 * 1024;
    const chunk = "a".repeat(4096);
    const src = ref("");
    const {
      ref: out,
      truncation,
      flush,
    } = useTailWindow(src, {
      maxLines: 5000,
      maxBytes,
    });

    for (let i = 0; i < 200; i++) {
      src.value += chunk;
      flush();
      expect(new TextEncoder().encode(out.value).length).toBeLessThanOrEqual(
        maxBytes,
      );
    }

    expect(out.value.endsWith(chunk)).toBe(true);
    expect(truncation.value).toBe("bytes");
  });

  it("连续 delta 合并到 80ms 窗口后一次刷新", () => {
    vi.useFakeTimers();
    const src = ref("A");
    const { ref: out } = useTailWindow(src, 5000);

    src.value = "AB";
    src.value = "ABC";
    expect(out.value).toBe("A");

    vi.advanceTimersByTime(79);
    expect(out.value).toBe("A");
    vi.advanceTimersByTime(1);
    expect(out.value).toBe("ABC");
  });
});
