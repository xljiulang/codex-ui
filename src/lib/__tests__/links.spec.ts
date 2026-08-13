import { describe, expect, it, vi, beforeEach } from "vitest";
import { reactive } from "vue";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve(null)),
}));

vi.mock("../../composables/useCodex", () => ({
  store: reactive({
    currentThreadCwd: null,
    server: { workspace: "D:/repo" },
    toast: "",
  }),
  toastError: (e: unknown) => String(e),
  resolveCwd: () => "D:/repo",
  setToast: (msg: string) => {
    store.toast = msg;
  },
}));

import { invoke } from "@tauri-apps/api/core";
import { localPathFromHref, openLink } from "../links";
import { store } from "../../composables/useCodex";

const mockedInvoke = vi.mocked(invoke);

describe("localPathFromHref 链接分类", () => {
  it("http/https 判定为网页", () => {
    expect(localPathFromHref("https://example.com/a?b=1", "D:/repo")).toEqual({
      kind: "web",
      url: "https://example.com/a?b=1",
    });
    expect(localPathFromHref("http://x", "D:/repo")).toEqual({
      kind: "web",
      url: "http://x",
    });
  });

  it("file:/// 解码并归一化为本地路径", () => {
    expect(localPathFromHref("file:///D:/a%20b.txt", "D:/repo")).toEqual({
      kind: "local",
      path: "D:\\a b.txt",
    });
  });

  it("/D:/、D:/、D:\\ 均判定为本地路径", () => {
    expect(localPathFromHref("/D:/x/y.md", "D:/repo")).toEqual({
      kind: "local",
      path: "D:\\x\\y.md",
    });
    expect(localPathFromHref("D:/x/y.md", "D:/repo")).toEqual({
      kind: "local",
      path: "D:\\x\\y.md",
    });
    expect(localPathFromHref("D:\\x\\y.md", "D:/repo")).toEqual({
      kind: "local",
      path: "D:\\x\\y.md",
    });
  });

  it("marked 编码形式（%5C）解码为本地路径", () => {
    expect(localPathFromHref("D:%5Ccodex%5Ca.md", "D:/repo")).toEqual({
      kind: "local",
      path: "D:\\codex\\a.md",
    });
    expect(localPathFromHref("D:%5C%E8%AE%A1%E5%88%92.md", "D:/repo")).toEqual({
      kind: "local",
      path: "D:\\计划.md",
    });
  });

  it("相对路径按工作目录解析（支持 ..）", () => {
    expect(localPathFromHref("src/a.ts", "D:/repo")).toEqual({
      kind: "local",
      path: "D:\\repo\\src\\a.ts",
    });
    expect(localPathFromHref("../b.ts", "D:/repo/src")).toEqual({
      kind: "local",
      path: "D:\\repo\\b.ts",
    });
  });

  it("mailto/#/空 无法分类返回 null", () => {
    expect(localPathFromHref("mailto:a@b.c", "D:/repo")).toBeNull();
    expect(localPathFromHref("#anchor", "D:/repo")).toBeNull();
    expect(localPathFromHref("", "D:/repo")).toBeNull();
    expect(localPathFromHref("javascript:alert(1)", "D:/repo")).toBeNull();
  });

  it("plugin:// URI 不作为本地路径", () => {
    expect(
      localPathFromHref(
        "plugin://documents@openai-primary-runtime",
        "D:/repo",
      ),
    ).toBeNull();
  });
});

describe("openLink 分发", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    store.toast = "";
  });

  it("网页链接调用 open_url", () => {
    openLink("https://example.com");
    expect(mockedInvoke).toHaveBeenCalledWith("open_url", {
      url: "https://example.com",
    });
  });

  it("本地链接调用 reveal_path", () => {
    openLink("file:///D:/a.txt");
    expect(mockedInvoke).toHaveBeenCalledWith("reveal_path", {
      path: "D:\\a.txt",
    });
  });

  it("相对链接按工作目录解析后调用 reveal_path", () => {
    openLink("src/a.ts");
    expect(mockedInvoke).toHaveBeenCalledWith("reveal_path", {
      path: "D:\\repo\\src\\a.ts",
    });
  });

  it("reveal_path 失败时 toast 提示", async () => {
    mockedInvoke.mockRejectedValueOnce("文件或目录不存在: D:\\a.txt");
    openLink("D:/a.txt");
    await vi.waitFor(() => {
      expect(store.toast).toContain("文件或目录不存在");
    });
  });

  it("无法分类的链接不调用任何命令", () => {
    openLink("mailto:a@b.c");
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("测试钩子开启时只记录分发、不真正调用命令", () => {
    const w = window as unknown as {
      __CODEX_UI_TEST__?: boolean;
      __CODEX_UI_TEST_LOG__?: { cmd: string }[];
    };
    w.__CODEX_UI_TEST__ = true;
    w.__CODEX_UI_TEST_LOG__ = [];
    openLink("https://example.com");
    openLink("file:///D:/a.txt");
    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(w.__CODEX_UI_TEST_LOG__?.map((l) => l.cmd)).toEqual([
      "open_url",
      "reveal_path",
    ]);
    w.__CODEX_UI_TEST__ = false;
  });
});
