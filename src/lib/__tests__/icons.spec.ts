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
      // 多路径 Logo 集合（SESSION_LOGO_PATHS）为数组，不作为单路径图标校验
      if (key === "SESSION_LOGO_PATHS") continue;
      expect(value, key).toBeTypeOf("string");
      expect(value.length).toBeGreaterThan(0);
    }
  });

  it("会话 Logo 多路径集合：六边形 + C 标记", () => {
    expect(icons.SESSION_LOGO_PATHS).toHaveLength(2);
    expect(icons.SESSION_LOGO_PATHS[0].d).toBe(icons.ICON_SESSION);
    expect(icons.SESSION_LOGO_PATHS[1]).toMatchObject({
      d: icons.ICON_SESSION_LOGO_C,
      accent: true,
    });
  });
});
