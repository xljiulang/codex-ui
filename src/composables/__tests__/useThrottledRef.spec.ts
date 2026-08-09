import { describe, expect, it, vi, afterEach } from "vitest";
import { ref } from "vue";
import { useThrottledRef } from "../useThrottledRef";

describe("useThrottledRef", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("源连续变化时最多每 intervalMs 刷新一次，取最新值", () => {
    vi.useFakeTimers();
    const source = ref("a");
    const { ref: target } = useThrottledRef(source, 80);
    expect(target.value).toBe("a");

    source.value = "b";
    source.value = "c";
    source.value = "d";
    expect(target.value).toBe("a");

    vi.advanceTimersByTime(80);
    expect(target.value).toBe("d");
  });

  it("flush 立即刷新为源最新值并取消挂起的定时器", () => {
    vi.useFakeTimers();
    const source = ref("a");
    const { ref: target, flush } = useThrottledRef(source, 80);
    source.value = "b";
    flush();
    expect(target.value).toBe("b");
    // 定时器已取消，不会再覆盖
    vi.advanceTimersByTime(200);
    expect(target.value).toBe("b");
  });
});
