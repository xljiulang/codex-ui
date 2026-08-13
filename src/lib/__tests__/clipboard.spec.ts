import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { copyText } from "../clipboard";

describe("copyText 剪贴板复制", () => {
  beforeEach(() => {
    if (!navigator.clipboard) {
      Object.defineProperty(navigator, "clipboard", {
        value: { writeText: vi.fn() },
        configurable: true,
      });
    }
    if (!("execCommand" in document)) {
      Object.defineProperty(document, "execCommand", {
        value: vi.fn(() => true),
        writable: true,
      });
    }
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("空文本直接返回 false", async () => {
    expect(await copyText("")).toBe(false);
  });

  it("navigator.clipboard 成功时返回 true", async () => {
    const write = vi
      .spyOn(navigator.clipboard, "writeText")
      .mockResolvedValue(undefined);
    expect(await copyText("abc")).toBe(true);
    expect(write).toHaveBeenCalledWith("abc");
  });

  it("clipboard 失败时降级 textarea + execCommand", async () => {
    vi.spyOn(navigator.clipboard, "writeText").mockRejectedValue(
      new Error("denied"),
    );
    const exec = vi.spyOn(document, "execCommand").mockReturnValue(true);
    expect(await copyText("abc")).toBe(true);
    expect(exec).toHaveBeenCalledWith("copy");
  });

  it("两级都失败时返回 false", async () => {
    vi.spyOn(navigator.clipboard, "writeText").mockRejectedValue(
      new Error("denied"),
    );
    vi.spyOn(document, "execCommand").mockImplementation(() => {
      throw new Error("no copy");
    });
    expect(await copyText("abc")).toBe(false);
  });
});
