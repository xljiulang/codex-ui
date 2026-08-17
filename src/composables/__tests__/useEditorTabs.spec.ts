import { beforeEach, describe, expect, it, vi } from "vitest";
import { EditorView } from "@codemirror/view";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
}));

const terminalEventsMock = vi.hoisted(() => ({
  ensureTerminalListeners: vi.fn(() => Promise.resolve()),
  attachTerminal: vi.fn(() => ({
    flush: () => [],
    exitCode: null,
    onData: vi.fn(),
    onExit: vi.fn(),
    detach: vi.fn(),
  })),
  releaseTerminal: vi.fn(),
}));

vi.mock("../useTerminalEvents", () => terminalEventsMock);

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import {
  __resetEditorTabsForTest,
  activeTabId,
  activateTab,
  closeAnyTab,
  closeAllOtherTabs,
  closeTabsToLeftAll,
  closeTabsToRightAll,
  closeTab,
  discardTabAndClose,
  openCommitFileDiffTab,
  openCommitTab,
  openDiffTab,
  openFileTab,
  openPreviewTab,
  openTerminalTab,
  pendingCloseId,
  saveFileTab,
  saveTabAndClose,
  tabs,
  type FileEditorTab,
  type PreviewEditorTab,
  type TerminalEditorTab,
} from "../useEditorTabs";
import {
  attachTerminal,
  ensureTerminalListeners,
  releaseTerminal,
} from "../useTerminalEvents";
import { settleConfirm, store } from "../useCodex";
import type { SessionTab } from "../useCodex";
import { insertTab } from "../useTabs";
import { TabIcon, TabKind } from "../../lib/tabs";

const mockedInvoke = vi.mocked(invoke);
const mockedConvertFileSrc = vi.mocked(convertFileSrc);
const mockedEnsureTerminalListeners = vi.mocked(ensureTerminalListeners);
const mockedAttachTerminal = vi.mocked(attachTerminal);
const mockedReleaseTerminal = vi.mocked(releaseTerminal);
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
    mockedEnsureTerminalListeners.mockReset();
    mockedAttachTerminal.mockReset();
    mockedReleaseTerminal.mockReset();
    __resetEditorTabsForTest();
    store.confirm = null;
  });

  it("初始不含固定会话标签（会话标签由 useCodex 管理）", () => {
    expect(tabs).toHaveLength(0);
    expect(activeTabId.value).toBe("");
  });

  it("打开文件创建标签并激活；重复打开只激活不重建", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    expect(tabs).toHaveLength(1);
    expect(activeTabId.value).not.toBe("");
    const tab = fileTab(activeTabId.value);
    expect(tab.title).toBe("a.txt");
    expect(tab.markdownPreview).toBe(false);
    expect(tab.loading).toBe(false);
    expect(tab.editorState).not.toBeNull();
    expect(tab.editorState!.doc.toString()).toBe("hello");

    await openFileTab(root, "a.txt");
    expect(tabs).toHaveLength(1);
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
      workspace: root,
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

  it("放弃并关闭：不写盘直接移除；未知标签 id 关闭无操作", async () => {
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
    expect(activeTabId.value).toBe("");

    closeTab("nope");
    expect(tabs).toHaveLength(0);
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
    expect(activeTabId.value).not.toBe("");
    const firstId = activeTabId.value;
    closeTab(firstId);
    expect(activeTabId.value).not.toBe(firstId);
    expect(activeTabId.value).not.toBe("");

    closeTab(activeTabId.value);
    expect(activeTabId.value).toBe("");
  });

  it("关闭其它所有标签：干净标签关闭，脏标签跳过并返回数量", async () => {
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
      workspace: root,
    });

    const skipped = closeAllOtherTabs();
    expect(skipped).toBe(1);
    expect(tabs.map((t) => t.id)).toEqual([dirtyTab.id]);
    expect(tabs.some((t) => t.kind === "diff")).toBe(false);
    expect(tabs.some((t) => t.title === "a.txt")).toBe(false);
    expect(tabs.some((t) => t.title === "b.txt")).toBe(false);
    expect(tabs.some((t) => t.id === dirtyTab.id)).toBe(true);
    view.destroy();
    host.remove();
  });

  it("关闭左边所有标签：仅关目标左侧，跳过脏标签并返回计数", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    await openFileTab(root, "b.txt");
    const dirtyTab = fileTab(activeTabId.value);
    const { view, host } = mountView(dirtyTab);
    view.dispatch({ changes: { from: 0, insert: "x" } });
    expect(dirtyTab.dirty).toBe(true);
    await openFileTab(root, "c.txt");
    const cTab = tabs.find((t) => t.title === "c.txt")!;

    const skipped = await closeTabsToLeftAll(cTab.id);
    expect(skipped).toBe(1);
    expect(tabs.map((t) => t.title)).toEqual(["b.txt", "c.txt"]);
    view.destroy();
    host.remove();
  });

  it("关闭右边所有标签：仅关目标右侧，保留目标本身", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    await openFileTab(root, "b.txt");
    await openFileTab(root, "c.txt");
    const aTab = tabs.find((t) => t.title === "a.txt")!;

    const skipped = await closeTabsToRightAll(aTab.id);
    expect(skipped).toBe(0);
    expect(tabs.map((t) => t.title)).toEqual(["a.txt"]);
  });

  it("关闭右边所有标签：跳过脏标签并计数", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    await openFileTab(root, "b.txt");
    const dirtyTab = fileTab(activeTabId.value);
    const { view, host } = mountView(dirtyTab);
    view.dispatch({ changes: { from: 0, insert: "x" } });
    expect(dirtyTab.dirty).toBe(true);
    await openFileTab(root, "c.txt");
    const aTab = tabs.find((t) => t.title === "a.txt")!;

    const skipped = await closeTabsToRightAll(aTab.id);
    expect(skipped).toBe(1);
    expect(tabs.map((t) => t.title)).toEqual(["a.txt", "b.txt"]);
    expect(tabs.some((t) => t.id === dirtyTab.id)).toBe(true);
    view.destroy();
    host.remove();
  });

  it("关闭左边所有标签：范围内的终端标签一并结束进程", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_kill") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    await openTerminalTab(root);
    await openFileTab(root, "b.txt");
    const bTab = tabs.find((t) => t.title === "b.txt")!;

    const skipped = await closeTabsToLeftAll(bTab.id);
    expect(skipped).toBe(0);
    // 终端恒在文件之前：b.txt 左侧含终端与 a.txt，一并关闭
    expect(tabs.map((t) => t.title)).toEqual(["b.txt"]);
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_kill", {
      id: expect.any(String),
    });
  });

  it("关闭左边：首个标签返回 0 且无变化", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    await openFileTab(root, "b.txt");
    const aTab = tabs.find((t) => t.title === "a.txt")!;

    expect(await closeTabsToLeftAll(aTab.id)).toBe(0);
    expect(tabs).toHaveLength(2);
  });

  it("关闭左边/右边：未知 id 返回 0", async () => {
    expect(await closeTabsToLeftAll("nope")).toBe(0);
    expect(await closeTabsToRightAll("nope")).toBe(0);
  });

  it("activateTab 忽略不存在的 id", () => {
    activateTab("nope");
    expect(activeTabId.value).toBe("");
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
      workspace: root,
    });
    expect(tabs).toHaveLength(1);
    const diffTabs = tabs.filter((t) => t.kind === "diff");
    expect(diffTabs).toHaveLength(1);
    expect(diffTabs[0]).toMatchObject({
      kind: "diff",
      loading: false,
      rows,
      brief: false,
    });
    expect(activeTabId.value).toBe(diffTabs[0].id);

    await openDiffTab({
      path: "a.txt",
      kind: "modify",
      diff: "diff --git a/a.txt b/a.txt\n@@ -1 +1 @@\n-a\n+b",
      workspace: root,
    });
    expect(tabs.filter((t) => t.kind === "diff")).toHaveLength(1);
  });

  it("打开图像预览：创建标签、生成 asset URL、激活；重复打开去重", async () => {
    await openPreviewTab("image", root, "pic.png");
    expect(tabs).toHaveLength(1);
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
        expect(args).toEqual({ workspace: root, path: "doc.pdf" });
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

  it("打开提交详情：创建标签、拉取详情、激活；重复打开去重", async () => {
    const detail = {
      hash: "a".repeat(40),
      shortHash: "aaaaaaa",
      subject: "feat: x",
      body: "feat: x\n\nbody",
      author: "t",
      authorEmail: "t@t",
      authorTimeSecs: 1000,
      committer: "t",
      committerEmail: "t@t",
      committerTimeSecs: 1000,
      parents: [],
      files: [],
    };
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_commit_detail") {
        return Promise.resolve(detail);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openCommitTab(root, detail.hash, "feat: x");
    expect(tabs).toHaveLength(1);
    const t = tabs.find((x) => x.kind === "commit");
    expect(t).toMatchObject({
      kind: "commit",
      workspace: root,
      hash: detail.hash,
      title: "feat: x",
      loading: false,
      error: "",
      detail,
    });
    expect(activeTabId.value).toBe(t!.id);
    expect(mockedInvoke).toHaveBeenCalledWith("git_changes_commit_detail", {
      workspace: root,
      hash: detail.hash,
    });

    await openCommitTab(root, detail.hash, "feat: x");
    expect(tabs.filter((x) => x.kind === "commit")).toHaveLength(1);
  });

  it("打开提交详情：读取失败保留标签并记录错误", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_commit_detail") {
        return Promise.reject("detail boom");
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openCommitTab(root, "a".repeat(40), "feat");
    const t = tabs.find((x) => x.kind === "commit");
    expect(t!.loading).toBe(false);
    expect(t!.error).toContain("detail boom");
  });

  it("打开提交文件 diff：创建 Diff 标签、拉取行数据、激活；按提交+文件去重", async () => {
    const rows = [
      { kind: "del", oldNo: 1, text: "x" },
      { kind: "add", newNo: 1, text: "y" },
    ];
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_commit_file_diff") {
        return Promise.resolve(rows);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const hash = "b".repeat(40);

    await openCommitFileDiffTab(root, hash, "src/a.txt", "modify");
    expect(tabs).toHaveLength(1);
    const t = tabs.find((x) => x.kind === "diff");
    expect(t).toMatchObject({
      kind: "diff",
      path: "src/a.txt",
      changeKind: "modify",
      workspace: root,
      loading: false,
      error: "",
      rows,
      title: "a.txt",
    });
    expect(activeTabId.value).toBe(t!.id);
    expect(mockedInvoke).toHaveBeenCalledWith("git_changes_commit_file_diff", {
      workspace: root,
      hash,
      path: "src/a.txt",
    });

    await openCommitFileDiffTab(root, hash, "src/a.txt", "modify");
    expect(tabs.filter((x) => x.kind === "diff")).toHaveLength(1);
    // 同一文件不同提交 → 独立 diff 标签
    await openCommitFileDiffTab(root, "c".repeat(40), "src/a.txt", "modify");
    expect(tabs.filter((x) => x.kind === "diff")).toHaveLength(2);
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
    expect(tabs).toHaveLength(0);
  });

  it("打开终端：创建标签并激活、调用 terminal_spawn", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openTerminalTab(root);
    expect(tabs).toHaveLength(1);
    const t = tabs.find(
      (x): x is TerminalEditorTab => x.kind === "terminal",
    );
    expect(t).toBeTruthy();
    expect(t!.workspace).toBe(root);
    expect(t!.title).toBe("cmd");
    expect(t!.loading).toBe(false);
    expect(t!.error).toBe("");
    expect(t!.exited).toBe(false);
    expect(t!.id).toMatch(/^terminal:/);
    expect(activeTabId.value).toBe(t!.id);
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_spawn", {
      id: t!.id,
      workspace: root,
    });
  });

  it("打开终端：设置 terminal_shell 为 powershell 时标题为 PowerShell", async () => {
    store.settings.terminal_shell = "powershell";
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openTerminalTab(root);
    const t = tabs.find(
      (x): x is TerminalEditorTab => x.kind === "terminal",
    );
    expect(t!.title).toBe("PowerShell");
    store.settings.terminal_shell = "cmd";
  });

  it("打开终端：先建立全局监听/缓冲，再发起 spawn", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openTerminalTab(root);
    const t = tabs.find(
      (x): x is TerminalEditorTab => x.kind === "terminal",
    );
    expect(t).toBeTruthy();
    expect(mockedEnsureTerminalListeners).toHaveBeenCalledTimes(1);
    expect(mockedAttachTerminal).toHaveBeenCalledWith(t!.id);
    const orderEnsure = mockedEnsureTerminalListeners.mock.invocationCallOrder[0];
    const orderAttach = mockedAttachTerminal.mock.invocationCallOrder[0];
    const orderSpawn = mockedInvoke.mock.invocationCallOrder[0];
    expect(orderEnsure).toBeLessThan(orderSpawn);
    expect(orderAttach).toBeLessThan(orderSpawn);
  });

  it("同目录连续打开生成不同 id（支持多开）", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openTerminalTab(root);
    await openTerminalTab(root);
    const ts = tabs.filter((x) => x.kind === "terminal");
    expect(ts).toHaveLength(2);
    expect(new Set(ts.map((t) => t.id)).size).toBe(2);
  });

  it("spawn 失败：标签保留并记录错误", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.reject("spawn boom");
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openTerminalTab(root);
    const t = tabs.find(
      (x): x is TerminalEditorTab => x.kind === "terminal",
    );
    expect(t!.loading).toBe(false);
    expect(t!.error).toContain("spawn boom");
  });

  it("关闭终端标签：调用 terminal_kill 并移除，不弹脏确认", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_kill") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openTerminalTab(root);
    const t = tabs.find(
      (x): x is TerminalEditorTab => x.kind === "terminal",
    );
    closeTab(t!.id);
    expect(pendingCloseId.value).toBeNull();
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_kill", {
      id: t!.id,
    });
    expect(tabs.some((x) => x.id === t!.id)).toBe(false);
  });

  it("关闭终端标签：释放事件桥缓冲", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_kill") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openTerminalTab(root);
    const t = tabs.find(
      (x): x is TerminalEditorTab => x.kind === "terminal",
    );
    closeTab(t!.id);
    expect(mockedReleaseTerminal).toHaveBeenCalledWith(t!.id);
  });

  it("关闭运行中的终端：先弹确认，确认后终止进程并移除", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_kill") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openTerminalTab(root);
    const t = tabs.find(
      (x): x is TerminalEditorTab => x.kind === "terminal",
    )!;
    t.busy = true;

    const pending = closeTab(t.id);
    expect(store.confirm?.title).toBe("关闭终端");
    expect(store.confirm?.message).toContain("终止该进程");
    expect(store.confirm?.confirmLabel).toBe("终止并关闭");

    settleConfirm(true);
    await pending;
    expect(tabs.some((x) => x.id === t.id)).toBe(false);
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_kill", { id: t.id });
  });

  it("关闭运行中的终端：取消确认则保留标签、不终止进程", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_kill") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openTerminalTab(root);
    const t = tabs.find(
      (x): x is TerminalEditorTab => x.kind === "terminal",
    )!;
    t.busy = true;

    const pending = closeTab(t.id);
    expect(store.confirm).not.toBeNull();
    settleConfirm(false);
    await pending;
    expect(tabs.some((x) => x.id === t.id)).toBe(true);
    expect(mockedInvoke).not.toHaveBeenCalledWith("terminal_kill", {
      id: t.id,
    });
  });

  it("关闭其它所有标签：跳过运行中的终端并计入返回计数", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_kill") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openTerminalTab(root);
    await openTerminalTab(root);
    const ts = tabs.filter(
      (x): x is TerminalEditorTab => x.kind === "terminal",
    );
    ts[0].busy = true;

    const skipped = closeAllOtherTabs();
    expect(skipped).toBe(1);
    expect(tabs.some((x) => x.id === ts[0].id)).toBe(true);
    expect(tabs.some((x) => x.id === ts[1].id)).toBe(false);
    expect(mockedInvoke).not.toHaveBeenCalledWith("terminal_kill", {
      id: ts[0].id,
    });
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_kill", {
      id: ts[1].id,
    });
  });

  it("关闭其它所有标签：未保存文件与运行中终端合并跳过计数", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_kill") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openFileTab(root, "a.txt");
    await openTerminalTab(root);
    const f = tabs.find(
      (x): x is FileEditorTab => x.kind === "file",
    )!;
    f.dirty = true;
    const t = tabs.find(
      (x): x is TerminalEditorTab => x.kind === "terminal",
    )!;
    t.busy = true;

    const skipped = closeAllOtherTabs();
    expect(skipped).toBe(2);
    expect(tabs.some((x) => x.id === f.id)).toBe(true);
    expect(tabs.some((x) => x.id === t.id)).toBe(true);
    expect(mockedInvoke).not.toHaveBeenCalledWith("terminal_kill", {
      id: t.id,
    });
  });

  it("关闭其它所有标签：终端一并结束进程", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_kill") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    await openTerminalTab(root);
    const skipped = closeAllOtherTabs();
    expect(skipped).toBe(0);
    expect(tabs).toHaveLength(0);
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_kill", {
      id: expect.any(String),
    });
  });

  it("启动期间关闭标签：spawn 完成后回收后端会话", async () => {
    let resolveSpawn!: (v: unknown) => void;
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") {
        return new Promise((r) => {
          resolveSpawn = r;
        });
      }
      if (cmd === "terminal_kill") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    const pending = openTerminalTab(root);
    // openTerminalTab 现在先 await 事件桥监听再 spawn，需让微任务队列推进一次
    await Promise.resolve();
    const t = tabs.find(
      (x): x is TerminalEditorTab => x.kind === "terminal",
    );
    expect(t).toBeTruthy();
    closeTab(t!.id);
    resolveSpawn({});
    await pending;
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_kill", {
      id: t!.id,
    });
    expect(tabs.some((x) => x.id === t!.id)).toBe(false);
  });
});

describe("closeAnyTab 统一关闭入口", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetEditorTabsForTest();
    tabs.splice(0, tabs.length);
    store.confirm = null;
  });

  it("会话标签：走 closeSessionTab 并移除", async () => {
    const tab: SessionTab = {
      id: "s1",
      kind: TabKind.Chat,
      title: "会话",
      icon: TabIcon.Chat,
      threadId: "t1",
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      taskMode: "execute",
      model: null,
      effort: null,
      draftJson: JSON.stringify({ type: "doc", content: [] }),
      draftAttachments: [],
      draftRefs: {},
      origin: "history",
      workspace: null,
      resumedThreadId: null,
      turnActive: false,
      currentTurnId: null,
      turnInterrupted: false,
      goalText: null,
      goalStatus: null,
      goalArmed: false,
      threadTokenUsage: null,
      followupQueue: [],
      attachments: [],
      planPrompt: null,
      plan: null,
      loading: false,
      newChatWorkspace: null,
      interactions: [],
    };
    tabs.push(tab);
    await closeAnyTab(tab);
    expect(tabs.some((t) => t.id === tab.id)).toBe(false);
  });

  it("运行中终端：确认后终止进程并移除", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_kill") return Promise.resolve({});
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await openTerminalTab(root);
    const t = tabs.find(
      (x): x is TerminalEditorTab => x.kind === TabKind.Terminal,
    )!;
    t.busy = true;
    const pending = closeAnyTab(t);
    expect(store.confirm?.title).toBe("关闭终端");
    settleConfirm(true);
    await pending;
    expect(tabs.some((x) => x.id === t.id)).toBe(false);
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_kill", { id: t.id });
  });

  it("脏文件：挂起确认不直接关闭", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await openFileTab(root, "a.txt");
    const f = tabs.find(
      (x): x is FileEditorTab => x.kind === TabKind.File,
    )!;
    f.dirty = true;
    await closeAnyTab(f);
    expect(pendingCloseId.value).toBe(f.id);
    expect(tabs.some((x) => x.id === f.id)).toBe(true);
  });

  function makeSessionTab(id: string, over: Partial<SessionTab> = {}): SessionTab {
    return {
      id,
      kind: "chat",
      title: "会话",
      icon: "chat",
      threadId: null,
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      taskMode: "execute",
      model: null,
      effort: null,
      draftJson: JSON.stringify({ type: "doc", content: [] }),
      draftAttachments: [],
      draftRefs: {},
      origin: null,
      workspace: null,
      resumedThreadId: null,
      turnActive: false,
      currentTurnId: null,
      turnInterrupted: false,
      goalText: null,
      goalStatus: null,
      goalArmed: false,
      threadTokenUsage: null,
      followupQueue: [],
      attachments: [],
      planPrompt: null,
      plan: null,
      loading: false,
      newChatWorkspace: null,
      interactions: [],
      ...over,
    };
  }

  it("统一列表三块排序：会话在前、终端居中、文件按打开顺序在后", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      if (cmd === "terminal_spawn") return Promise.resolve({});
      return Promise.resolve(undefined);
    });

    insertTab(makeSessionTab("s1"));
    await openTerminalTab(root);
    await openFileTab(root, "a.txt");
    await openFileTab(root, "b.txt");
    // 后插入的会话仍回到会话块末尾（终端之前）
    insertTab(makeSessionTab("s2"));
    expect(tabs.map((t) => t.title)).toEqual([
      "会话",
      "会话",
      "cmd",
      "a.txt",
      "b.txt",
    ]);
  });

  it("closeTab 关闭会话标签：路由到会话关闭并切换活动标签", async () => {
    insertTab(makeSessionTab("s1"));
    insertTab(makeSessionTab("s2"));
    activeTabId.value = "s1";

    await closeTab("s1");
    expect(tabs.some((t) => t.id === "s1")).toBe(false);
    expect(activeTabId.value).toBe("s2");
  });

  it("批量关闭（关闭左边）范围内的运行中会话：跳过并计数", async () => {
    insertTab(makeSessionTab("s1", { turnActive: true }));
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await openFileTab(root, "a.txt");

    const aTab = tabs.find((t) => t.title === "a.txt")!;
    const skipped = await closeTabsToLeftAll(aTab.id);
    expect(skipped).toBe(1);
    expect(tabs.some((t) => t.id === "s1")).toBe(true);
  });
});
