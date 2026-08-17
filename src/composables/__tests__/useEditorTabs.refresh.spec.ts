import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises } from "@vue/test-utils";

const docxMock = vi.hoisted(() => ({
  docxToHtml: vi.fn(),
  jsonToDocx: vi.fn(),
  isDocxPath: vi.fn((name: string) => name.toLowerCase().endsWith(".docx")),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("../../lib/docx", () => docxMock);

import { invoke } from "@tauri-apps/api/core";
import {
  __resetEditorTabsForTest,
  activateTab,
  openDocxTab,
  openFileTab,
  openPreviewTab,
  tabs,
  type DocxEditorTab,
  type FileEditorTab,
  type PreviewEditorTab,
} from "../useEditorTabs";
import {
  __resetRefreshForTest,
  __setRefreshInteractionForTest,
  refreshActiveTabFromFs,
} from "../useEditorTabs/refresh";
import { activeTabId } from "../useTabs";
import { docxToHtml } from "../../lib/docx";

const mockedInvoke = vi.mocked(invoke);
const mockedDocxToHtml = vi.mocked(docxToHtml);
const root = "D:\\repo";

function fileContent(content: string, validUtf8 = true) {
  return { content, validUtf8, byteSize: content.length };
}

function binaryContent(bytes: string) {
  return new TextEncoder().encode(bytes).buffer;
}

/** 资源树打开的真实形态：标签 path 为绝对路径，事件 paths 为相对路径 */
function absPath(path: string): string {
  return root + "\\" + path;
}

async function openFile(path: string, content: string): Promise<FileEditorTab> {
  mockedInvoke.mockImplementation((cmd) => {
    if (cmd === "session_fs_read") {
      return Promise.resolve(fileContent(content));
    }
    return Promise.reject(new Error(`unexpected ${cmd}`));
  });
  await openFileTab(root, absPath(path));
  const tab = tabs.find(
    (t): t is FileEditorTab => t.kind === "file" && t.path === absPath(path),
  )!;
  expect(tab).toBeTruthy();
  return tab;
}

describe("refreshActiveTabFromFs 活动标签外部刷新", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedDocxToHtml.mockReset();
    __resetEditorTabsForTest();
    __resetRefreshForTest();
  });

  afterEach(() => {
    __resetRefreshForTest();
  });

  it("文本标签内容变化：重建状态、保留光标、复位脏标记并提示", async () => {
    const tab = await openFile("a.txt", "line1\nline2\nline3");
    tab.cursor = { line: 2, col: 1 };

    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("line1\nline2\nchanged\nline4"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await refreshActiveTabFromFs({ root, paths: ["a.txt"] });

    expect(tab.editorState!.doc.toString()).toBe("line1\nline2\nchanged\nline4");
    expect(tab.editorState!.selection.main.head).toBe(6); // line2 行首
    expect(tab.dirty).toBe(false);
    expect(tab.status).toContain("已从磁盘刷新");
  });

  it("文本标签内容相同：跳过重建，不写状态提示", async () => {
    const tab = await openFile("a.txt", "hello");
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await refreshActiveTabFromFs({ root, paths: ["a.txt"] });
    expect(tab.editorState!.doc.toString()).toBe("hello");
    expect(tab.status).toBe("");
  });

  it("脏标签跳过：不读盘并提示外部已变更", async () => {
    const tab = await openFile("a.txt", "hello");
    tab.dirty = true;
    mockedInvoke.mockClear();
    await refreshActiveTabFromFs({ root, paths: ["a.txt"] });
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_read",
      expect.anything(),
    );
    expect(tab.status).toContain("未保存修改");
  });

  it("只刷新活动标签：非活动标签即使路径命中也不刷新", async () => {
    await openFile("a.txt", "A");
    await openFile("b.txt", "B");
    const a = tabs.find(
      (t): t is FileEditorTab => t.kind === "file" && t.path === absPath("a.txt"),
    )!;
    mockedInvoke.mockClear();
    // 活动标签是 b.txt，payload 命中 a.txt：不触发任何读取
    await refreshActiveTabFromFs({ root, paths: ["a.txt"] });
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_read",
      expect.anything(),
    );
    expect(a.editorState!.doc.toString()).toBe("A");

    // 活动标签命中：只读 b.txt
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("B2"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await refreshActiveTabFromFs({ root, paths: ["b.txt"] });
    const b = tabs.find(
      (t): t is FileEditorTab => t.kind === "file" && t.path === absPath("b.txt"),
    )!;
    expect(b.editorState!.doc.toString()).toBe("B2");
  });

  it("事件把非活动匹配标签标记 stale，不匹配路径/工作区不标记", async () => {
    await openFile("a.txt", "A");
    await openFile("b.txt", "B");
    const a = tabs.find(
      (t): t is FileEditorTab => t.kind === "file" && t.path === absPath("a.txt"),
    )!;
    const b = tabs.find(
      (t): t is FileEditorTab => t.kind === "file" && t.path === absPath("b.txt"),
    )!;
    mockedInvoke.mockClear();

    await refreshActiveTabFromFs({ root, paths: ["a.txt"] });
    expect(a.stale).toBe(true); // 非活动命中：标记待补刷
    expect(b.stale).toBe(false); // 活动标签 b 未命中路径

    // 工作区不匹配：不标记
    await refreshActiveTabFromFs({ root: "D:\\other", paths: ["a.txt"] });
    expect(a.stale).toBe(true); // 维持此前标记
  });

  it("活动标签事件刷新成功后清除 stale", async () => {
    await openFile("a.txt", "A");
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("A2"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await refreshActiveTabFromFs({ root, paths: ["a.txt"] });
    const a = tabs.find(
      (t): t is FileEditorTab => t.kind === "file" && t.path === absPath("a.txt"),
    )!;
    expect(a.editorState!.doc.toString()).toBe("A2");
    expect(a.stale).toBe(false);
  });

  it("stale 标签切回活动触发重载并清除标记", async () => {
    await openFile("a.txt", "A");
    await openFile("b.txt", "B");
    const a = tabs.find(
      (t): t is FileEditorTab => t.kind === "file" && t.path === absPath("a.txt"),
    )!;
    mockedInvoke.mockClear();
    await refreshActiveTabFromFs({ root, paths: ["a.txt"] }); // 非活动 a 标记 stale
    expect(a.stale).toBe(true);

    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("A2"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    activeTabId.value = a.id;
    await flushPromises();
    expect(a.editorState!.doc.toString()).toBe("A2");
    expect(a.stale).toBe(false);
  });

  it("stale + 未保存修改的标签切回活动：不重载且清除标记", async () => {
    await openFile("a.txt", "A");
    await openFile("b.txt", "B");
    const a = tabs.find(
      (t): t is FileEditorTab => t.kind === "file" && t.path === absPath("a.txt"),
    )!;
    mockedInvoke.mockClear();
    await refreshActiveTabFromFs({ root, paths: ["a.txt"] });
    a.dirty = true;

    activeTabId.value = a.id;
    await flushPromises();
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_read",
      expect.anything(),
    );
    expect(a.editorState!.doc.toString()).toBe("A");
    expect(a.stale).toBe(false);
  });

  it("stale 标签激活刷新失败：保留内容并清除标记", async () => {
    await openFile("a.txt", "A");
    await openFile("b.txt", "B");
    const a = tabs.find(
      (t): t is FileEditorTab => t.kind === "file" && t.path === absPath("a.txt"),
    )!;
    mockedInvoke.mockClear();
    await refreshActiveTabFromFs({ root, paths: ["a.txt"] });
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.reject(new Error("文件不存在"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    activeTabId.value = a.id;
    await flushPromises();
    expect(a.editorState!.doc.toString()).toBe("A");
    expect(a.status).toContain("外部刷新失败");
    expect(a.stale).toBe(false);
  });

  it("预览标签 stale 切回活动：更新 imageUrl 击穿缓存", async () => {
    await openFile("a.txt", "A");
    mockedInvoke.mockResolvedValue(undefined);
    await openPreviewTab("image", root, "pic.png");
    const img = tabs.find(
      (t): t is PreviewEditorTab => t.kind === "preview" && t.path === "pic.png",
    )!;
    const a = tabs.find(
      (t): t is FileEditorTab => t.kind === "file" && t.path === absPath("a.txt"),
    )!;
    activeTabId.value = a.id; // 预览转非活动
    await flushPromises();
    mockedInvoke.mockClear();
    await refreshActiveTabFromFs({ root, paths: ["pic.png"] });
    expect(img.stale).toBe(true);

    activeTabId.value = img.id;
    await flushPromises();
    expect(img.imageUrl).toMatch(/asset:\/\/pic\.png\?t=\d+/);
    expect(img.stale).toBe(false);
  });

  it("工作区不匹配跳过", async () => {
    const tab = await openFile("a.txt", "hello");
    mockedInvoke.mockClear();
    await refreshActiveTabFromFs({
      root: "D:\\other",
      paths: ["a.txt"],
    });
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_read",
      expect.anything(),
    );
    expect(tab.editorState!.doc.toString()).toBe("hello");
  });

  it("读取失败保留内容并提示", async () => {
    const tab = await openFile("a.txt", "hello");
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.reject(new Error("文件不存在"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await refreshActiveTabFromFs({ root, paths: ["a.txt"] });
    expect(tab.editorState!.doc.toString()).toBe("hello");
    expect(tab.status).toContain("外部刷新失败");
  });

  it("docx 标签内容变化：setContent 并恢复选区、复位脏标记", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read_bytes") {
        return Promise.resolve(binaryContent("PK1"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    mockedDocxToHtml.mockResolvedValue({ html: "<p>A</p>", warnings: [] });
    await openDocxTab(root, absPath("a.docx"));
    const tab = tabs.find(
      (t): t is DocxEditorTab =>
        t.kind === "docx" && t.path === absPath("a.docx"),
    )!;
    const setContent = vi.fn();
    const setTextSelection = vi.fn();
    tab.editor = {
      state: { selection: { from: 5 }, doc: { content: { size: 100 } } },
      commands: { setContent, setTextSelection },
      view: { dom: { closest: () => null } },
    } as unknown as DocxEditorTab["editor"];

    mockedDocxToHtml.mockResolvedValue({ html: "<p>B</p>", warnings: [] });
    await refreshActiveTabFromFs({ root, paths: ["a.docx"] });
    expect(setContent).toHaveBeenCalledWith("<p>B</p>");
    expect(setTextSelection).toHaveBeenCalledWith(5);
    expect(tab.initialHtml).toBe("<p>B</p>");
    expect(tab.dirty).toBe(false);
  });

  it("docx 标签内容相同：不重写编辑器", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read_bytes") {
        return Promise.resolve(binaryContent("PK1"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    mockedDocxToHtml.mockResolvedValue({ html: "<p>A</p>", warnings: [] });
    await openDocxTab(root, absPath("a.docx"));
    const tab = tabs.find(
      (t): t is DocxEditorTab =>
        t.kind === "docx" && t.path === absPath("a.docx"),
    )!;
    const setContent = vi.fn();
    tab.editor = {
      state: { selection: { from: 0 }, doc: { content: { size: 10 } } },
      commands: { setContent, setTextSelection: vi.fn() },
      view: { dom: { closest: () => null } },
    } as unknown as DocxEditorTab["editor"];

    await refreshActiveTabFromFs({ root, paths: ["a.docx"] });
    expect(setContent).not.toHaveBeenCalled();
    expect(tab.initialHtml).toBe("<p>A</p>");
  });

  it("图片预览：imageUrl 带时间戳击穿缓存", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    await openPreviewTab("image", root, "pic.png");
    const tab = tabs.find(
      (t): t is PreviewEditorTab => t.kind === "preview" && t.path === "pic.png",
    )!;
    expect(tab.previewType).toBe("image");
    await refreshActiveTabFromFs({ root, paths: ["pic.png"] });
    expect(tab.imageUrl).toMatch(/asset:\/\/pic\.png\?t=\d+/);
  });

  it("PDF 预览：替换 pdfData 字节", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read_bytes") {
        return Promise.resolve(binaryContent("AAA"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await openPreviewTab("pdf", root, "doc.pdf");
    const tab = tabs.find(
      (t): t is PreviewEditorTab => t.kind === "preview" && t.path === "doc.pdf",
    )!;
    expect(tab.pdfData!.length).toBe(3);

    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read_bytes") {
        return Promise.resolve(binaryContent("BBBB"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await refreshActiveTabFromFs({ root, paths: ["doc.pdf"] });
    expect(tab.pdfData!.length).toBe(4);
  });

  it("XLSX 预览：替换 xlsxData 字节", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read_bytes") {
        return Promise.resolve(binaryContent("AAA"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await openPreviewTab("xlsx", root, "book.xlsx");
    const tab = tabs.find(
      (t): t is PreviewEditorTab =>
        t.kind === "preview" && t.path === "book.xlsx",
    )!;
    expect(tab.xlsxData!.length).toBe(3);

    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read_bytes") {
        return Promise.resolve(binaryContent("BBBB"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await refreshActiveTabFromFs({ root, paths: ["book.xlsx"] });
    expect(tab.xlsxData!.length).toBe(4);
  });

  it("XLSX 预览非活动标签：标记 stale，切回活动补刷并清除标记", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read_bytes") {
        return Promise.resolve(binaryContent("AAA"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await openPreviewTab("xlsx", root, "book.xlsx");
    const tab = tabs.find(
      (t): t is PreviewEditorTab =>
        t.kind === "preview" && t.path === "book.xlsx",
    )!;
    await openFile("a.txt", "A");
    expect(tab.stale).toBe(false);
    expect(tab.xlsxData!.length).toBe(3);

    // 非活动 xlsx 命中事件：只标记 stale，不读盘
    mockedInvoke.mockClear();
    await refreshActiveTabFromFs({ root, paths: ["book.xlsx"] });
    expect(tab.stale).toBe(true);
    expect(tab.xlsxData!.length).toBe(3);
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_read_bytes",
      expect.anything(),
    );

    // 切回活动：自动补刷替换字节并清除标记
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read_bytes") {
        return Promise.resolve(binaryContent("BBBB"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    activateTab(tab.id);
    await flushPromises();
    expect(tab.xlsxData!.length).toBe(4);
    expect(tab.stale).toBe(false);
  });

  it("编辑区交互后延迟到空闲再刷新", async () => {
    const tab = await openFile("a.txt", "hello");
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("world"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    mockedInvoke.mockClear();

    // 模拟约 100ms 前的交互：还需等约 900ms
    __setRefreshInteractionForTest(Date.now() + 100);
    const p = refreshActiveTabFromFs({ root, paths: ["a.txt"] });
    await flushPromises();
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_read",
      expect.anything(),
    );
    await new Promise((r) => setTimeout(r, 1100));
    await p;
    expect(tab.editorState!.doc.toString()).toBe("world");
    expect(tab.status).toContain("已从磁盘刷新");
  });
});
