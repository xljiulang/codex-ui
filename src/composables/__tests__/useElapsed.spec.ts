import { describe, expect, it, vi } from "vitest";

const { holder } = vi.hoisted(() => ({
  holder: {} as { ref?: { value: number } },
}));

vi.mock("../useClock", async () => {
  const { ref } = await import("vue");
  const nowRef = ref(1000);
  holder.ref = nowRef;
  return { useClock: () => nowRef };
});

import { useElapsed } from "../useElapsed";

describe("useElapsed 实时计时", () => {
  it("elapsed = max(0, now - startAtMs)", () => {
    const { elapsed } = useElapsed(500);
    expect(elapsed.value).toBe(500);
    holder.ref!.value = 300;
    expect(elapsed.value).toBe(0);
    holder.ref!.value = 2000;
    expect(elapsed.value).toBe(1500);
  });

  it("起始时间与当前时间相等时为 0", () => {
    holder.ref!.value = 42;
    const { elapsed } = useElapsed(42);
    expect(elapsed.value).toBe(0);
  });
});
