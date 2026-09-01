import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  availableBundledTools,
  bundledToolsDeveloperInstructions,
  loadBundledTools,
} from "../useBundledTools";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const mockedInvoke = vi.mocked(invoke);

describe("useBundledTools", () => {
  beforeEach(() => {
    availableBundledTools.value = [];
    mockedInvoke.mockReset();
  });

  it("loadBundledTools 成功：填充 ref", async () => {
    mockedInvoke.mockResolvedValue(["ast-grep", "fd", "rg"]);
    await loadBundledTools();
    expect(availableBundledTools.value).toEqual(["ast-grep", "fd", "rg"]);
    expect(mockedInvoke).toHaveBeenCalledWith("cli_tools_available");
  });

  it("loadBundledTools 失败：清空 ref", async () => {
    mockedInvoke.mockRejectedValue(new Error("boom"));
    await loadBundledTools();
    expect(availableBundledTools.value).toEqual([]);
  });

  it("无可用工具时不注入（返回 null）", () => {
    expect(bundledToolsDeveloperInstructions()).toBeNull();
  });

  it("命中工具时生成包含可用工具的说明，不含未命中工具", () => {
    availableBundledTools.value = ["rg", "fd"];
    const s = bundledToolsDeveloperInstructions()!;
    expect(s).toContain("# Collaboration Mode: Default");
    expect(s).toContain("rg：文本搜索");
    expect(s).toContain("fd：按名快速查找文件/目录");
    expect(s).not.toContain("ast-grep");
  });

  it("未知工具退化为基础名", () => {
    availableBundledTools.value = ["my-tool"];
    const s = bundledToolsDeveloperInstructions()!;
    expect(s).toContain("- my-tool");
  });
});
