import { describe, expect, it } from "vitest";
import { formatDuration, formatElapsed } from "../format";

describe("耗时格式化", () => {
  it("进行中计时 mm:ss.s", () => {
    expect(formatElapsed(0)).toBe("00:00.0");
    expect(formatElapsed(-5)).toBe("00:00.0");
    expect(formatElapsed(3_200)).toBe("00:03.2");
    expect(formatElapsed(75_400)).toBe("01:15.4");
  });

  it("超过 10 分钟用 mm:ss", () => {
    expect(formatElapsed(10 * 60 * 1000 + 500)).toBe("10:00");
  });

  it("完成耗时显示秒", () => {
    expect(formatDuration(1_234)).toBe("1.2s");
    expect(formatDuration(12_345)).toBe("12.3s");
  });

  it("超过 10 分钟沿用 mm:ss", () => {
    expect(formatDuration(10 * 60 * 1000)).toBe("10:00");
  });
});
