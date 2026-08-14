import { describe, expect, it } from "vitest";
import * as icons from "../icons";

describe("icons 图标常量", () => {
  it("导出目标旗子图标 ICON_GOAL（目标芯片依赖）", () => {
    expect(icons.ICON_GOAL).toContain("M4 3h2v18H4z");
  });

  it("全部图标常量为非空字符串", () => {
    const entries = Object.entries(icons);
    expect(entries.length).toBeGreaterThan(10);
    for (const [key, value] of entries) {
      expect(value, key).toBeTypeOf("string");
      expect(value.length).toBeGreaterThan(0);
    }
  });
});
