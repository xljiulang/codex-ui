import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../useCodex")>();
  return {
    ...mod,
    resolveSessionWorkspace: vi.fn(() => "D:\\repo"),
    workspace: { value: "D:\\repo" },
    setToast: vi.fn(),
  };
});

import { invoke } from "@tauri-apps/api/core";
import { useMentionFileSearch } from "../useMentionFileSearch";

const mockedInvoke = vi.mocked(invoke);

describe("useMentionFileSearch", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("scheduleFileSearch 防抖后发起 fuzzyFileSearch 并填充结果", async () => {
    mockedInvoke.mockResolvedValueOnce({
      files: [{ path: "D:\\repo\\a.txt" }],
    });
    const s = useMentionFileSearch();
    s.scheduleFileSearch("a");
    expect(mockedInvoke).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(250);
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "fuzzyFileSearch",
      params: {
        query: "a",
        roots: ["D:\\repo"],
        cancellationToken: null,
      },
    });
    expect(s.fileResults.value).toEqual([{ path: "D:\\repo\\a.txt" }]);
    expect(s.searchingFiles.value).toBe(false);
  });

  it("空 token 立即重置且不发起请求", async () => {
    const s = useMentionFileSearch();
    s.scheduleFileSearch("x");
    s.resetFileSearch(true);
    await vi.advanceTimersByTimeAsync(300);
    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(s.fileResults.value).toEqual([]);
  });

  it("resetFileSearch 使进行中的慢响应失效", async () => {
    let resolveSearch: (v: { files: { path: string }[] }) => void = () => {};
    mockedInvoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveSearch = resolve;
        }),
    );
    const s = useMentionFileSearch();
    s.scheduleFileSearch("a");
    await vi.advanceTimersByTimeAsync(250);
    expect(s.searchingFiles.value).toBe(true);
    s.resetFileSearch(true);
    resolveSearch({ files: [{ path: "D:\\repo\\stale.txt" }] });
    await vi.advanceTimersByTimeAsync(0);
    expect(s.fileResults.value).toEqual([]); // 旧序号结果被丢弃
    expect(s.searchingFiles.value).toBe(false);
  });

  it("搜索失败时清空结果并提示", async () => {
    mockedInvoke.mockRejectedValueOnce(new Error("boom"));
    const s = useMentionFileSearch();
    s.scheduleFileSearch("a");
    await vi.advanceTimersByTimeAsync(250);
    expect(s.fileResults.value).toEqual([]);
    expect(s.searchingFiles.value).toBe(false);
  });
});
