import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { debounce } from "../debounce";

describe("debounce 防抖", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("多次调用只保留最后一次，延迟后触发", () => {
    const fn = vi.fn();
    const d = debounce(fn, 300);
    d.run(1);
    d.run(2);
    d.run(3);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(299);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(fn).toHaveBeenCalledTimes(1);
    expect(fn).toHaveBeenCalledWith(3);
  });

  it("cancel 取消未执行的调用", () => {
    const fn = vi.fn();
    const d = debounce(fn, 300);
    d.run();
    d.cancel();
    vi.advanceTimersByTime(400);
    expect(fn).not.toHaveBeenCalled();
  });
});
