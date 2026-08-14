import { beforeEach, describe, expect, it, vi } from "vitest";
import { EditorView } from "@codemirror/view";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
}));

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import {
  __resetEditorTabsForTest,
  activeTabId,
  activateTab,
  closeAllOtherTabs,
  closeTab,
  discardTabAndClose,
  openDiffTab,
  openFileTab,
  openPreviewTab,
  pendingCloseId,
  saveFileTab,
  saveTabAndClose,
  tabs,
  type FileEditorTab,
  type PreviewEditorTab,
} from "../useEditorTabs";

const mockedInvoke = vi.mocked(invoke);
const mockedConvertFileSrc = vi.mocked(convertFileSrc);
const root = "D:\\repo";

function fileContent(content: string, validUtf8 = true) {
  return {
    content,
    validUtf8,
    byteSize: content.length,
  };
}

function fileTab(id: string): FileEditorTab {
  const t = tabs.find(
    (it): it is FileEditorTab => it.kind === "file" && it.id === id,
  );
  if (!t) throw new Error(`file tab not found: ${id}`);
  return t;
}

function mountView(tab: FileEditorTab): { view: EditorView; host: HTMLDivElement } {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const view = new EditorView({ state: tab.editorState!, parent: host });
  return { view, host };
}

describe("useEditorTabs 标签状态", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetEditorTabsForTest();
  });

  it("初始仅含“对话”主标签且不可关闭", () => {
    expect(tabs).toHaveLength(1);
    expect(tabs[0]).toMatchObject({ kind: "chat", id: "chat", title: "对话" });
    expect(activeTabId.value).toBe("chat");
  });

  it("打开文件创建标签并激活；重复打开只激活不重建", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    expect(tabs).toHaveLength(2);
    expect(activeTabId.value).not.toBe("chat");
    const tab = fileTab(activeTabId.value);
    expect(tab.title).toBe("a.txt");
    expect(tab.loading).toBe(false);
    expect(tab.editorState).not.toBeNull();
    expect(tab.editorState!.doc.toString()).toBe("hello");

    await openFileTab(root, "a.txt");
    expect(tabs).toHaveLength(2);
    expect(activeTabId.value).toBe(tab.id);
  });

  it("编辑后 dirty 为真；保存调用 session_fs_write 并还原 CRLF/BOM", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("\uFEFFa\r\nb\r\n"));
      }
      if (cmd === "session_fs_write") {
        return Promise.resolve({});
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    const tab = fileTab(activeTabId.value);
    expect(tab.eol).toBe("\r\n");
    expect(tab.hadBom).toBe(true);
    const { view, host } = mountView(tab);
    view.dispatch({
      changes: { from: 0, insert: "x" },
      selection: { anchor: 1 },
    });
    expect(tab.dirty).toBe(true);

    expect(await saveFileTab(tab.id)).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_write", {
      root,
      path: "a.txt",
      content: "\uFEFFxa\r\nb\r\n",
    });
    expect(tab.dirty).toBe(false);
    expect(tab.status).toContain("已保存");
    view.destroy();
    host.remove();
  });

  it("非 UTF-8 文件以只读方式打开", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("abc", false));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    const tab = fileTab(activeTabId.value);
    expect(tab.readOnly).toBe(true);
    expect(await saveFileTab(tab.id)).toBe(false);
  });

  it("读取失败：标签保留并记录错误", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.reject("read boom");
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    const tab = fileTab(activeTabId.value);
    expect(tab.loading).toBe(false);
    expect(tab.error).toContain("read boom");
  });

  it("关闭脏标签先挂起确认；放弃后移除，保存并关闭先写盘再移除", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      if (cmd === "session_fs_write") {
        return Promise.resolve({});
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    const tab = fileTab(activeTabId.value);
    const { view, host } = mountView(tab);
    view.dispatch({ changes: { from: 0, insert: "x" } });
    expect(tab.dirty).toBe(true);

    closeTab(tab.id);
    expect(pendingCloseId.value).toBe(tab.id);
    expect(tabs.some((t) => t.id === tab.id)).toBe(true);

    await saveTabAndClose(tab.id);
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_write", expect.anything());
    expect(tabs.some((t) => t.id === tab.id)).toBe(false);
    expect(pendingCloseId.value).toBeNull();
    view.destroy();
    host.remove();
  });

  it("放弃并关闭：不写盘直接移除；对话标签不可关闭", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    const tab = fileTab(activeTabId.value);
    const { view, host } = mountView(tab);
    view.dispatch({ changes: { from: 0, insert: "x" } });

    closeTab(tab.id);
    discardTabAndClose(tab.id);
    expect(tabs.some((t) => t.id === tab.id)).toBe(false);
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_write",
      expect.anything(),
    );
    expect(activeTabId.value).toBe("chat");

    closeTab("chat");
    expect(tabs[0].id).toBe("chat");
    view.destroy();
    host.remove();
  });

  it("关闭活动标签后激活相邻标签", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    await openFileTab(root, "b.txt");
    expect(activeTabId.value).not.toBe("chat");
    const firstId = activeTabId.value;
    closeTab(firstId);
    expect(activeTabId.value).not.toBe(firstId);
    expect(activeTabId.value).not.toBe("chat");

    closeTab(activeTabId.value);
    expect(activeTabId.value).toBe("chat");
  });

  it("关闭其它所有标签：保留会话标签，干净标签关闭，脏标签跳过并返回数量", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      if (cmd === "build_diff_preview") {
        return Promise.resolve([]);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    await openFileTab(root, "b.txt");
    await openFileTab(root, "c.txt");
    const dirtyTab = fileTab(activeTabId.value);
    const { view, host } = mountView(dirtyTab);
    view.dispatch({ changes: { from: 0, insert: "x" } });
    expect(dirtyTab.dirty).toBe(true);

    await openDiffTab({
      path: "d.txt",
      kind: "add",
      diff: "diff --git a/d.txt b/d.txt\n@@ -0,0 +1 @@\n+x",
      workspace_root: root,
    });

    const skipped = closeAllOtherTabs();
    expect(skipped).toBe(1);
    expect(tabs.map((t) => t.id)).toEqual(["chat", dirtyTab.id]);
    expect(tabs.some((t) => t.kind === "diff")).toBe(false);
    expect(tabs.some((t) => t.title === "a.txt")).toBe(false);
    expect(tabs.some((t) => t.title === "b.txt")).toBe(false);
    expect(tabs.some((t) => t.id === dirtyTab.id)).toBe(true);
    view.destroy();
    host.remove();
  });

  it("activateTab 忽略不存在的 id", () => {
    activateTab("nope");
    expect(activeTabId.value).toBe("chat");
  });

  it("打开 diff 标签：拉取行数据、激活、重复打开去重", async () => {
    const rows = [{ kind: "ctx", oldNo: 1, newNo: 1, text: "a" }];
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "build_diff_preview") {
        return Promise.resolve(rows);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openDiffTab({
      path: "a.txt",
      kind: "modify",
      diff: "diff --git a/a.txt b/a.txt\n@@ -1 +1 @@\n-a\n+b",
      workspace_root: root,
    });
    expect(tabs).toHaveLength(2);
    const diffTabs = tabs.filter((t) => t.kind === "diff");
    expect(diffTabs).toHaveLength(1);
    expect(diffTabs[0]).toMatchObject({ kind: "diff", loading: false, rows });
    expect(activeTabId.value).toBe(diffTabs[0].id);

    await openDiffTab({
      path: "a.txt",
      kind: "modify",
      diff: "diff --git a/a.txt b/a.txt\n@@ -1 +1 @@\n-a\n+b",
      workspace_root: root,
    });
    expect(tabs.filter((t) => t.kind === "diff")).toHaveLength(1);
  });

  it("打开图像预览：创建标签、生成 asset URL、激活；重复打开去重", async () => {
    await openPreviewTab("image", root, "pic.png");
    expect(tabs).toHaveLength(2);
    const preview = tabs.find(
      (t): t is PreviewEditorTab => t.kind === "preview",
    );
    expect(preview).toBeTruthy();
    expect(preview!.previewType).toBe("image");
    expect(preview!.loading).toBe(false);
    expect(preview!.error).toBe("");
    expect(preview!.imageUrl).toBe("asset://pic.png");
    expect(mockedConvertFileSrc).toHaveBeenCalledWith("pic.png");
    expect(activeTabId.value).toBe(preview!.id);

    await openPreviewTab("image", root, "pic.png");
    expect(tabs.filter((t) => t.kind === "preview")).toHaveLength(1);
    expect(activeTabId.value).toBe(preview!.id);
  });

  it("打开 PDF 预览：读取 session_fs_read_bytes 并解码为字节", async () => {
    mockedInvoke.mockImplementation((cmd, args) => {
      if (cmd === "session_fs_read_bytes") {
        expect(args).toEqual({ root, path: "doc.pdf" });
        return Promise.resolve({ content: "aGVsbG8=", byteSize: 5 });
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openPreviewTab("pdf", root, "doc.pdf");
    const preview = tabs.find(
      (t): t is PreviewEditorTab => t.kind === "preview",
    );
    expect(preview).toBeTruthy();
    expect(preview!.previewType).toBe("pdf");
    expect(preview!.loading).toBe(false);
    expect(preview!.error).toBe("");
    expect(Array.from(preview!.pdfData ?? [])).toEqual([
      104, 101, 108, 108, 111,
    ]);
    expect(activeTabId.value).toBe(preview!.id);
  });

  it("PDF 读取失败：标签保留并记录错误", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read_bytes") {
        return Promise.reject("pdf boom");
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openPreviewTab("pdf", root, "doc.pdf");
    const preview = tabs.find(
      (t): t is PreviewEditorTab => t.kind === "preview",
    );
    expect(preview!.loading).toBe(false);
    expect(preview!.error).toContain("pdf boom");
    expect(preview!.pdfData).toBeNull();
  });

  it("预览标签关闭与全部关闭：无脏确认直接移除", async () => {
    await openPreviewTab("image", root, "pic.png");
    await openPreviewTab("pdf", root, "doc.pdf");
    const previews = tabs.filter((t) => t.kind === "preview");
    expect(previews).toHaveLength(2);

    closeTab(previews[0].id);
    expect(tabs.some((t) => t.id === previews[0].id)).toBe(false);

    const skipped = closeAllOtherTabs();
    expect(skipped).toBe(0);
    expect(tabs.filter((t) => t.kind === "preview")).toHaveLength(0);
    expect(tabs.map((t) => t.id)).toEqual(["chat"]);
  });
});
