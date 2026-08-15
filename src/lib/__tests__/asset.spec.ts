import { describe, expect, it, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: vi.fn(),
}));

import { convertFileSrc } from "@tauri-apps/api/core";
import { assetUrl } from "../asset";

const mockedConvert = vi.mocked(convertFileSrc);

describe("assetUrl", () => {
  beforeEach(() => {
    mockedConvert.mockReset();
  });

  it("转换成功返回 asset URL", () => {
    mockedConvert.mockReturnValue("asset://D:/a.png");
    expect(assetUrl("D:\\a.png")).toBe("asset://D:/a.png");
    expect(mockedConvert).toHaveBeenCalledWith("D:\\a.png");
  });

  it("转换抛错时回退原路径", () => {
    mockedConvert.mockImplementation(() => {
      throw new Error("boom");
    });
    expect(assetUrl("D:\\a.png")).toBe("D:\\a.png");
  });
});
