import { describe, expect, it } from "vitest";
import {
  formatChatTime,
  formatDuration,
  formatElapsed,
  formatRelativeTime,
  formatTokens,
  relPathOf,
} from "../format";

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

  it("超过 30 天显示两位数年份的紧凑日期（适配固定时间列）", () => {
    const past = Date.now() / 1000 - 40 * 86400;
    expect(formatRelativeTime(past)).toMatch(/^\d{2}\/\d{1,2}\/\d{1,2}$/);
  });
});

describe("formatChatTime 消息时间", () => {
  const now = new Date(2026, 8, 3, 9, 0).getTime();

  it("当天仅显示 HH:mm", () => {
    expect(formatChatTime(new Date(2026, 8, 3, 14, 5).getTime(), now)).toBe(
      "14:05",
    );
  });

  it("同年非当天带 M月D日", () => {
    expect(formatChatTime(new Date(2026, 7, 9, 14, 5).getTime(), now)).toBe(
      "8月9日 14:05",
    );
  });

  it("跨年补年份", () => {
    expect(formatChatTime(new Date(2025, 11, 31, 23, 59).getTime(), now)).toBe(
      "2025年12月31日 23:59",
    );
  });
});

describe("relPathOf", () => {
  it("root 与 path 分隔符不一致时仍正确剥离", () => {
    expect(
      relPathOf("D:/codex/codex-ui", "D:\\codex\\codex-ui\\src\\components"),
    ).toBe("src\\components");
    expect(relPathOf("D:\\repo", "D:/repo/src/a.ts")).toBe("src\\a.ts");
  });

  it("大小写不一致时仍正确剥离", () => {
    expect(relPathOf("d:/repo/", "D:\\Repo\\src\\a.ts")).toBe("src\\a.ts");
  });

  it("path 等于 root 返回空串", () => {
    expect(relPathOf("D:\\repo", "D:\\repo")).toBe("");
    expect(relPathOf("D:/repo", "d:\\repo")).toBe("");
  });

  it("非 root 下路径原样返回", () => {
    expect(relPathOf("D:\\repo", "D:\\other\\a.ts")).toBe("D:\\other\\a.ts");
  });
});

describe("formatTokens", () => {
  it("小于 1万 原样返回", () => {
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(9_999)).toBe("9999");
  });

  it("1万 及以上一位小数万（.0 省略）", () => {
    expect(formatTokens(10_000)).toBe("1万");
    expect(formatTokens(12_345)).toBe("1.2万");
    expect(formatTokens(123_456)).toBe("12.3万");
    expect(formatTokens(1_234_567)).toBe("123.5万");
  });

  it("1亿 及以上一位小数亿（.0 省略）", () => {
    expect(formatTokens(100_000_000)).toBe("1亿");
    expect(formatTokens(150_000_000)).toBe("1.5亿");
    expect(formatTokens(200_000_000)).toBe("2亿");
  });
});
