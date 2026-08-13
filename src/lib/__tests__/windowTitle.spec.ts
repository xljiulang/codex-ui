import { describe, expect, it, vi } from "vitest";

const setTitle = vi.fn(() => Promise.resolve());

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTitle }),
}));

import { setWindowTitleFromPath } from "../windowTitle";

describe("setWindowTitleFromPath 窗口标题", () => {
  it("用文件名（不含路径）设置标题", async () => {
    await setWindowTitleFromPath("D:/repo/src/app.ts");
    expect(setTitle).toHaveBeenCalledWith("app.ts");
  });

  it("非 Tauri 环境异常静默忽略", async () => {
    setTitle.mockRejectedValueOnce(new Error("no tauri"));
    await expect(setWindowTitleFromPath("D:/a.txt")).resolves.toBeUndefined();
  });
});
