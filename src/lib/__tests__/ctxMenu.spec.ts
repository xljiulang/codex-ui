import { afterEach, describe, expect, it, vi } from "vitest";
import { clampMenuPos } from "../ctxMenu";

describe("clampMenuPos 右键菜单定位", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("视口足够大：保持点击位置", () => {
    vi.spyOn(window, "innerWidth", "get").mockReturnValue(1200);
    vi.spyOn(window, "innerHeight", "get").mockReturnValue(800);
    expect(clampMenuPos(300, 200, 160, 92)).toEqual({ x: 300, y: 200 });
  });

  it("菜单超出右侧/底部时左移/上移收进视口", () => {
    vi.spyOn(window, "innerWidth", "get").mockReturnValue(400);
    vi.spyOn(window, "innerHeight", "get").mockReturnValue(300);
    const pos = clampMenuPos(380, 280, 160, 92);
    expect(pos.x).toBeLessThanOrEqual(400 - 160 - 4);
    expect(pos.y).toBeLessThanOrEqual(300 - 92 - 4);
  });

  it("窗口比菜单还小时贴边显示，坐标不为负", () => {
    vi.spyOn(window, "innerWidth", "get").mockReturnValue(100);
    vi.spyOn(window, "innerHeight", "get").mockReturnValue(60);
    expect(clampMenuPos(90, 50, 160, 92)).toEqual({ x: 4, y: 4 });
  });
});
