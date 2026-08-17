import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { nextTick } from "vue";

const docxMock = vi.hoisted(() => ({
  docxToHtml: vi.fn(),
  jsonToDocx: vi.fn(),
  isDocxPath: vi.fn((name: string) => name.toLowerCase().endsWith(".docx")),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
}));
vi.mock("../../lib/docx", () => docxMock);
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { invoke } from "@tauri-apps/api/core";
import { store } from "../useCodex";
import {
  __resetSessionFsForTest,
  childrenByPath,
  copyBuffer,
  createTextFile,
  ensureEntryIcons,
  expanded,
  iconCacheKey,
  iconFor,
  loadingRoot,
  openPathInApp,
  pasteAvailable,
  revealAbsPathInTree,
  rootEntry,
  selectedPath,
  setSessionFsActive,
} from "../useSessionFs";
import {
  __resetEditorTabsForTest,
  tabs,
  type FileEditorTab,
} from "../useEditorTabs";
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
    store.workspace = null;
    store.server.startupWorkspace = root;
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

it("激活资源面板：session_fs_watch_start 传字符串工作区而非 computed（防循环引用）", async () => {
  store.server.startupWorkspace = root;
  store.workspace = null;
  mockedInvoke.mockImplementation((cmd) => {
    if (cmd === "session_fs_metadata") {
      return Promise.resolve({
        name: "repo",
        path: root,
        relPath: ".",
        isDir: true,
        size: null,
        modifiedAtMs: 0,
        createdAtMs: 0,
        childCount: 0,
      });
    }
    if (cmd === "session_fs_list") return Promise.resolve([]);
    if (cmd === "session_fs_watch_start") return Promise.resolve(undefined);
    return Promise.resolve(undefined);
  });
  setSessionFsActive(true);
  await vi.waitFor(
    () => {
      expect(mockedInvoke).toHaveBeenCalledWith("session_fs_watch_start", {
        workspace: root,
      });
    },
    { timeout: 3000, interval: 20 },
  );
  const call = mockedInvoke.mock.calls.find(
    ([c]) => c === "session_fs_watch_start",
  );
  expect(typeof (call?.[1] as { workspace?: unknown }).workspace).toBe(
    "string",
  );
  __resetSessionFsForTest();
});

describe("openPathInApp 对话链接应用内打开", () => {
  beforeEach(() => {
    __resetEditorTabsForTest();
    store.server.startupWorkspace = root;
    store.toast = "";
    mockedInvoke.mockClear();
  });

  it("工作区内文本文件：probe 后打开文件标签", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_probe_text") return Promise.resolve(true);
      if (cmd === "session_fs_read") {
        return Promise.resolve({ content: "hello", validUtf8: true, byteSize: 5 });
      }
      return Promise.resolve(undefined);
    });
    const path = root + "\\a.txt";
    const ok = await openPathInApp(path);
    expect(ok).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_probe_text", {
      workspace: root,
      path,
    });
    const tab = tabs.find(
      (t): t is FileEditorTab => t.kind === "file" && t.path === path,
    );
    expect(tab).toBeTruthy();
    expect(tab?.workspace).toBe(root);
  });

  it("工作区内 PDF：打开 PDF 预览标签", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read_bytes") {
        return Promise.resolve({ content: btoa("x"), size: 1 });
      }
      return Promise.resolve(undefined);
    });
    const path = root + "\\a.pdf";
    const ok = await openPathInApp(path);
    expect(ok).toBe(true);
    const tab = tabs.find((t) => t.kind === "preview" && t.path === path);
    expect(tab).toBeTruthy();
    expect((tab as { previewType?: string } | undefined)?.previewType).toBe(
      "pdf",
    );
  });

  it("工作区内图像：打开图像预览标签（asset 协议）", async () => {
    const path = root + "\\a.png";
    const ok = await openPathInApp(path);
    expect(ok).toBe(true);
    const tab = tabs.find((t) => t.kind === "preview" && t.path === path);
    expect(tab).toBeTruthy();
    expect((tab as { previewType?: string } | undefined)?.previewType).toBe(
      "image",
    );
  });

  it("工作区内 .docx：直接打开富文本编辑标签（不做文本探测）", async () => {
    docxMock.docxToHtml.mockResolvedValue({ html: "<p>doc</p>", warnings: [] });
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read_bytes") {
        return Promise.resolve({ content: btoa("PK"), byteSize: 2 });
      }
      return Promise.resolve(undefined);
    });
    const path = root + "\\a.docx";
    const ok = await openPathInApp(path);
    expect(ok).toBe(true);
    expect(mockedInvoke).not.toHaveBeenCalledWith("session_fs_probe_text", {
      workspace: root,
      path,
    });
    const tab = tabs.find((t) => t.kind === "docx" && t.path === path);
    expect(tab).toBeTruthy();
    await vi.waitFor(
      () => {
        expect(
          (tab as { initialHtml?: string | null } | undefined)?.initialHtml,
        ).toBe("<p>doc</p>");
      },
      { timeout: 3000, interval: 20 },
    );
  });

  it("二进制文件：返回 false 且不打开标签", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_probe_text") return Promise.resolve(false);
      return Promise.resolve(undefined);
    });
    const ok = await openPathInApp(root + "\\a.bin");
    expect(ok).toBe(false);
    expect(
      tabs.some((t) => t.kind === "file" || t.kind === "preview"),
    ).toBe(false);
  });

  it("探测失败（目录/缺失）：返回 false 且不 toast", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_probe_text") {
        return Promise.reject(new Error("不是文件"));
      }
      return Promise.resolve(undefined);
    });
    const ok = await openPathInApp(root + "\\src");
    expect(ok).toBe(false);
    expect(store.toast).toBe("");
  });

  it("工作区外文件：以父目录为根打开", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_probe_text") return Promise.resolve(true);
      if (cmd === "session_fs_read") {
        return Promise.resolve({ content: "hi", validUtf8: true, byteSize: 2 });
      }
      return Promise.resolve(undefined);
    });
    const path = "D:\\other\\x.txt";
    const ok = await openPathInApp(path);
    expect(ok).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_probe_text", {
      workspace: "D:\\other",
      path: "x.txt",
    });
    const tab = tabs.find(
      (t): t is FileEditorTab => t.kind === "file" && t.path === "x.txt",
    );
    expect(tab).toBeTruthy();
    expect(tab?.workspace).toBe("D:\\other");
  });

  it("测试钩子开启时短路返回 false 且不调 IPC", async () => {
    const w = window as unknown as { __CODEX_UI_TEST__?: boolean };
    w.__CODEX_UI_TEST__ = true;
    try {
      const ok = await openPathInApp(root + "\\a.txt");
      expect(ok).toBe(false);
      expect(mockedInvoke).not.toHaveBeenCalled();
    } finally {
      w.__CODEX_UI_TEST__ = false;
    }
  });
});

describe("useSessionFs 粘贴可用性与新建文本文件", () => {
  beforeEach(() => {
    __resetSessionFsForTest();
    store.server.startupWorkspace = root;
    store.toast = "";
    copyBuffer.value = [];
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue([]);
  });

  afterEach(() => {
    __resetSessionFsForTest();
  });

  it("pasteAvailable：无复制记录且剪贴板无文件时为 false", async () => {
    expect(await pasteAvailable()).toBe(false);
    expect(mockedInvoke).toHaveBeenCalledWith("clipboard_file_paths");
  });

  it("pasteAvailable：系统剪贴板有文件时为 true", async () => {
    mockedInvoke.mockResolvedValue(["D:\\src\\a.txt"]);
    expect(await pasteAvailable()).toBe(true);
  });

  it("pasteAvailable：内部复制记录非空时为 true，不再查剪贴板", async () => {
    copyBuffer.value = ["D:\\src\\a.txt"];
    expect(await pasteAvailable()).toBe(true);
    expect(mockedInvoke).not.toHaveBeenCalledWith("clipboard_file_paths");
  });

  it("pasteAvailable：剪贴板读取失败按不可用处理", async () => {
    mockedInvoke.mockRejectedValue(new Error("剪贴板忙"));
    expect(await pasteAvailable()).toBe(false);
  });

  it("createTextFile：调用命令、提示并刷新", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "session_fs_create_file") {
        return Promise.resolve({
          name: "新建文本文件.txt",
          path: `${root}\\新建文本文件.txt`,
          relPath: "新建文本文件.txt",
          isDir: false,
          size: 0,
          modifiedAtMs: 0,
          createdAtMs: 0,
          childCount: null,
        });
      }
      if (cmd === "session_fs_metadata") {
        return Promise.resolve({
          name: "repo",
          path: root,
          relPath: ".",
          isDir: true,
          size: null,
          modifiedAtMs: 0,
          createdAtMs: 0,
          childCount: 0,
        });
      }
      if (cmd === "session_fs_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    await createTextFile(root + "\\src");

    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_create_file", {
      workspace: root,
      dir: root + "\\src",
    });
    expect(store.toast).toContain("已创建「新建文本文件.txt」");
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_metadata", {
      workspace: root,
      path: root,
    });
  });
});

describe("useSessionFs 工作区切换保留旧数据", () => {
  const rootEntryData: FsEntry = {
    name: "repo",
    path: root,
    relPath: ".",
    isDir: true,
    size: null,
    modifiedAtMs: 0,
    createdAtMs: 0,
    childCount: 1,
  };

  beforeEach(() => {
    __resetSessionFsForTest();
    store.workspace = null;
    store.server.startupWorkspace = root;
    store.toast = "";
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "session_fs_metadata") return Promise.resolve(rootEntryData);
      if (cmd === "session_fs_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
  });

  afterEach(() => {
    __resetSessionFsForTest();
  });

  it("切换工作区不清空旧数据，新根加载完成后更新并清理旧根缓存", async () => {
    setSessionFsActive(true);
    await flushPromises();
    expect(rootEntry.value?.path).toBe(root);

    let resolveMeta!: (v: unknown) => void;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "session_fs_metadata") {
        return new Promise((r) => {
          resolveMeta = r;
        });
      }
      if (cmd === "session_fs_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    const other = "D:\\other";
    store.workspace = other;
    await flushPromises();
    // 新根 metadata 挂起中：旧 rootEntry 仍在、loading 不影响旧数据展示
    expect(rootEntry.value?.path).toBe(root);
    expect(loadingRoot.value).toBe(true);

    const otherEntry: FsEntry = {
      ...rootEntryData,
      name: "other",
      path: other,
    };
    resolveMeta(otherEntry);
    await flushPromises();
    await flushPromises();
    expect(rootEntry.value?.path).toBe(other);
    // 旧根缓存被清理
    expect(childrenByPath[root]).toBeUndefined();
    expect(store.toast).toBe("");
  });

  it("metadata 挂起期间再次切换工作区：旧结果作废不写入", async () => {
    setSessionFsActive(true);
    await flushPromises();
    expect(rootEntry.value?.path).toBe(root);

    let resolveB!: (v: unknown) => void;
    let resolveC!: (v: unknown) => void;
    let metaCall = 0;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "session_fs_metadata") {
        metaCall += 1;
        if (metaCall === 1) {
          return new Promise((r) => {
            resolveB = r;
          });
        }
        return new Promise((r) => {
          resolveC = r;
        });
      }
      if (cmd === "session_fs_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    const b = "D:\\b";
    const c = "D:\\c";
    store.workspace = b;
    await flushPromises();
    store.workspace = c;
    await flushPromises();

    resolveB({ ...rootEntryData, name: "b", path: b });
    await flushPromises();
    expect(rootEntry.value?.path).toBe(root);

    resolveC({ ...rootEntryData, name: "c", path: c });
    await flushPromises();
    await flushPromises();
    expect(rootEntry.value?.path).toBe(c);
  });
});

describe("revealAbsPathInTree 资源树定位", () => {
  const rootEntryData: FsEntry = {
    name: "repo",
    path: root,
    relPath: ".",
    isDir: true,
    size: null,
    modifiedAtMs: 0,
    createdAtMs: 0,
    childCount: 2,
  };
  const srcEntry = file("src", "src", true);
  const mainEntry = file("main.ts", "src/main.ts");

  beforeEach(() => {
    __resetSessionFsForTest();
    store.workspace = root;
    store.server.startupWorkspace = root;
    store.toast = "";
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((cmd: string, args) => {
      if (cmd === "session_fs_metadata") return Promise.resolve(rootEntryData);
      if (cmd === "session_fs_list") {
        const dir = (args as { dir?: string } | undefined)?.dir;
        if (dir === root) return Promise.resolve([srcEntry]);
        if (dir === root + "\\src") return Promise.resolve([mainEntry]);
        return Promise.resolve([]);
      }
      return Promise.resolve(undefined);
    });
  });

  afterEach(() => {
    __resetSessionFsForTest();
  });

  it("根未加载时先加载根，再展开祖先并选中目标文件", async () => {
    expect(rootEntry.value).toBeNull();
    expect(childrenByPath[root]).toBeUndefined();

    await revealAbsPathInTree(root + "\\src\\main.ts");
    await nextTick();

    // 根由定位流程补齐加载
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_metadata", {
      workspace: root,
      path: root,
    });
    expect(rootEntry.value?.path).toBe(root);
    expect(childrenByPath[root]?.map((e) => e.path)).toEqual([srcEntry.path]);
    expect(childrenByPath[root + "\\src"]?.map((e) => e.path)).toEqual([
      mainEntry.path,
    ]);
    expect(expanded.has(root)).toBe(true);
    expect(expanded.has(root + "\\src")).toBe(true);
    expect(selectedPath.value).toBe(root + "\\src\\main.ts");
  });

  it("根已加载时不重复拉取根元信息", async () => {
    setSessionFsActive(true);
    await flushPromises();
    expect(rootEntry.value?.path).toBe(root);
    mockedInvoke.mockClear();

    await revealAbsPathInTree(root + "\\src\\main.ts");
    await nextTick();

    expect(mockedInvoke).not.toHaveBeenCalledWith("session_fs_metadata", {
      workspace: root,
      path: root,
    });
    expect(selectedPath.value).toBe(root + "\\src\\main.ts");
    expect(expanded.has(root + "\\src")).toBe(true);
  });

  it("并发 reveal 同一根：session_fs_metadata 只调用一次", async () => {
    let resolveMeta!: (v: unknown) => void;
    let metaCalls = 0;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "session_fs_metadata") {
        metaCalls += 1;
        return new Promise((r) => {
          resolveMeta = r;
        });
      }
      if (cmd === "session_fs_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    const p1 = revealAbsPathInTree(root + "\\src\\main.ts");
    const p2 = revealAbsPathInTree(root + "\\src\\main.ts");
    // 单飞：并发定位共享同一次根加载
    expect(metaCalls).toBe(1);

    resolveMeta(rootEntryData);
    await p1;
    await p2;
    await nextTick();
    expect(metaCalls).toBe(1);
    expect(rootEntry.value?.path).toBe(root);
    expect(selectedPath.value).toBe(root + "\\src\\main.ts");
  });

  it("正斜杠 workspace（如 git repo_workspace）：缓存键全为反斜杠规范，无正斜杠平行键", async () => {
    store.workspace = "D:/repo";
    await revealAbsPathInTree("D:/repo/src/main.ts");
    await nextTick();

    expect(rootEntry.value?.path).toBe(root);
    expect(childrenByPath[root]).toBeDefined();
    expect(childrenByPath["D:/repo"]).toBeUndefined();
    expect(childrenByPath[root + "\\src"]).toBeDefined();
    expect(expanded.has(root)).toBe(true);
    expect(expanded.has(root + "\\src")).toBe(true);
    expect(expanded.has("D:/repo")).toBe(false);
    expect(selectedPath.value).toBe(root + "\\src\\main.ts");
  });
});
