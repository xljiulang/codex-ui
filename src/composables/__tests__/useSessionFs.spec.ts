import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { invoke } from "@tauri-apps/api/core";
import { store } from "../useCodex";
import {
  __resetSessionFsForTest,
  ensureEntryIcons,
  iconCacheKey,
  iconFor,
} from "../useSessionFs";
import type { FsEntry } from "../../lib/sessionFs";

const mockedInvoke = vi.mocked(invoke);
const root = "D:\\repo";

function file(name: string, relPath: string, isDir = false): FsEntry {
  return {
    name,
    path: `${root}\\${relPath.replace("/", "\\")}`,
    relPath,
    isDir,
    size: isDir ? null : 1,
    modifiedAtMs: 0,
    createdAtMs: 0,
    childCount: isDir ? 0 : null,
  };
}

describe("useSessionFs 文件图标缓存", () => {
  beforeEach(() => {
    __resetSessionFsForTest();
    store.server.workspace = root;
    mockedInvoke.mockClear();
    mockedInvoke.mockResolvedValue([]);
  });
  afterEach(() => {
    __resetSessionFsForTest();
  });

  it("iconCacheKey：按扩展名/无扩展名路径/目录", () => {
    expect(iconCacheKey(file("a.txt", "a.txt"))).toBe("ext:.txt");
    expect(iconCacheKey(file("README", "README"))).toBe("file:README");
    expect(iconCacheKey(file("src", "src", true))).toBeNull();
  });

  it("ensureEntryIcons：同扩展名去重只发一次请求，结果回填缓存", async () => {
    const a = file("a.txt", "a.txt");
    const b = file("b.txt", "sub/b.txt");
    mockedInvoke.mockImplementation((cmd, args) => {
      if (cmd === "session_fs_icons") {
        const req = (args as { requests: { path: string }[] }).requests;
        return Promise.resolve(
          req.map((r) => ({ path: r.path, dataUri: "data:image/png;base64,AAAA" })),
        );
      }
      return Promise.resolve(undefined);
    });

    await ensureEntryIcons([a, b]);

    expect(mockedInvoke).toHaveBeenCalledTimes(1);
    const [cmd, args] = mockedInvoke.mock.calls[0];
    expect(cmd).toBe("session_fs_icons");
    expect((args as { requests: unknown[] }).requests).toHaveLength(1);
    expect(iconFor(a)).toBe("data:image/png;base64,AAAA");
    expect(iconFor(b)).toBe("data:image/png;base64,AAAA");
  });

  it("失败项缓存 null 且不再请求", async () => {
    const a = file("a.txt", "a.txt");
    mockedInvoke.mockResolvedValue([{ path: a.path, dataUri: null }]);

    await ensureEntryIcons([a]);
    expect(iconFor(a)).toBeNull();
    await ensureEntryIcons([a]);
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
  });

  it("整批失败不缓存，下次可重试", async () => {
    const a = file("a.txt", "a.txt");
    mockedInvoke.mockRejectedValueOnce("boom");

    await ensureEntryIcons([a]);
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
    expect(iconFor(a)).toBeNull();

    mockedInvoke.mockResolvedValueOnce([
      { path: a.path, dataUri: "data:image/png;base64,BBBB" },
    ]);
    await ensureEntryIcons([a]);
    expect(iconFor(a)).toBe("data:image/png;base64,BBBB");
  });

  it("请求未返回的键置 null", async () => {
    const a = file("a.txt", "a.txt");
    mockedInvoke.mockResolvedValue([]);

    await ensureEntryIcons([a]);
    expect(iconFor(a)).toBeNull();
  });
});
