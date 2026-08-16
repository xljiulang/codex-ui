import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return {
    ...mod,
    refreshThreads: vi.fn(),
  };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import ResourceView from "../ResourceView.vue";
import {
  registerComposerAddHandler,
  store,
  unregisterComposerAddHandler,
  __resetSessionTabsForTest,
} from "../../composables/useCodex";
import {
  __resetSessionFsForTest,
  selectedPath,
} from "../../composables/useSessionFs";
import {
  __resetGitChangesForTest,
  gitRevealTarget,
} from "../../composables/useGitChanges";
import {
  __resetEditorTabsForTest,
  activeTabId,
  openDiffTab,
  openFileTab,
  tabs,
  type FileEditorTab,
  type PreviewEditorTab,
} from "../../composables/useEditorTabs";
import { tooltipDirective } from "../../directives/tooltip";
import type { FsEntry } from "../../lib/sessionFs";
import { ICON_AT } from "../../lib/icons";
import { TabIcon, TabKind } from "../../lib/tabs";

const mockedInvoke = vi.mocked(invoke);
const mockedConvertFileSrc = vi.mocked(convertFileSrc);
const rootPath = "D:\\codex\\codex-ui";

const rootEntry: FsEntry = {
  name: "codex-ui",
  path: rootPath,
  relPath: ".",
  isDir: true,
  size: null,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: 3,
};
const srcDir: FsEntry = {
  name: "src",
  path: rootPath + "\\src",
  relPath: "src",
  isDir: true,
  size: null,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: 1,
};
const nodeModules: FsEntry = {
  name: "node_modules",
  path: rootPath + "\\node_modules",
  relPath: "node_modules",
  isDir: true,
  size: null,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: 0,
};
const aTxt: FsEntry = {
  name: "a.txt",
  path: rootPath + "\\a.txt",
  relPath: "a.txt",
  isDir: false,
  size: 1536,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: null,
};
const mainTs: FsEntry = {
  name: "main.ts",
  path: rootPath + "\\src\\main.ts",
  relPath: "src/main.ts",
  isDir: false,
  size: 2048,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: null,
};
const picPng: FsEntry = {
  name: "pic.png",
  path: rootPath + "\\src\\pic.png",
  relPath: "src/pic.png",
  isDir: false,
  size: 2048,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: null,
};
const docPdf: FsEntry = {
  name: "doc.pdf",
  path: rootPath + "\\src\\doc.pdf",
  relPath: "src/doc.pdf",
  isDir: false,
  size: 4096,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: null,
};
/** 右键菜单剪贴板文件源（默认含文件，粘贴可见；置空验证隐藏） */
let clipboardFiles: string[] = [];
/** 「新建文件夹」测试态：创建后加入 src 目录列表，驱动行内重命名渲染 */
let createdFolder: FsEntry | null = null;
/** 「新建文本文件」测试态：创建后加入 src 目录列表，驱动行内重命名渲染 */
let createdTextFile: FsEntry | null = null;
/** 「新建文本文件」菜单系统图标（默认可用；置 null 验证回退 SVG） */
let txtIconUri: string | null = "data:image/png;base64,TXTICON";

function mockFs() {
  mockedInvoke.mockImplementation((cmd, args) => {
    if (cmd === "clipboard_file_paths") return Promise.resolve(clipboardFiles);
    if (cmd === "session_fs_icon_for_ext") return Promise.resolve(txtIconUri);
    if (cmd === "session_fs_metadata") {
      const path = (args as { path?: string }).path;
      if (path === aTxt.path) return Promise.resolve(aTxt);
      return Promise.resolve(rootEntry);
    }
    if (cmd === "session_fs_list") {
      const dir = (args as { dir?: string }).dir;
      if (dir === rootPath) return Promise.resolve([srcDir, nodeModules, aTxt]);
      if (dir === srcDir.path)
        return Promise.resolve([
          mainTs,
          picPng,
          docPdf,
          ...(createdFolder ? [createdFolder] : []),
          ...(createdTextFile ? [createdTextFile] : []),
        ]);
      return Promise.resolve([]);
    }
    if (cmd === "session_fs_search") {
      const query = (args as { query?: string }).query ?? "";
      if (query.toLowerCase().includes("main")) return Promise.resolve([mainTs]);
      if (query.toLowerCase().includes("src")) return Promise.resolve([srcDir]);
      if (query.toLowerCase().includes("doc")) return Promise.resolve([docPdf]);
      return Promise.resolve([]);
    }
    if (cmd === "session_fs_icons") return Promise.resolve([]);
    if (cmd === "session_fs_probe_text") {
      const path = (args as { path?: string }).path;
      return Promise.resolve(path !== picPng.path);
    }
    if (cmd === "session_fs_read_bytes") {
      return Promise.resolve({ content: "JVBERi0x", byteSize: 8 });
    }
    if (cmd === "session_fs_read") {
      return Promise.resolve({ content: "hello", validUtf8: true, byteSize: 5 });
    }
    if (cmd === "startup_workspace") return Promise.resolve(rootPath);
    if (cmd === "session_fs_rename") return Promise.resolve({ ...aTxt, name: "b.txt" });
    if (cmd === "session_fs_create_dir") {
      createdFolder = {
        name: "新建文件夹",
        path: srcDir.path + "\\新建文件夹",
        relPath: "src/新建文件夹",
        isDir: true,
        size: null,
        modifiedAtMs: 0,
        createdAtMs: 0,
        childCount: 0,
      };
      return Promise.resolve(createdFolder);
    }
    if (cmd === "session_fs_create_file") {
      createdTextFile = {
        name: "新建文本文件.txt",
        path: srcDir.path + "\\新建文本文件.txt",
        relPath: "src/新建文本文件.txt",
        isDir: false,
        size: 0,
        modifiedAtMs: 0,
        createdAtMs: 0,
        childCount: null,
      };
      return Promise.resolve(createdTextFile);
    }
    if (cmd === "session_fs_delete") return Promise.resolve(undefined);
    if (cmd === "session_fs_paste") return Promise.resolve([mainTs]);
    if (
      cmd === "session_fs_watch_start" ||
      cmd === "session_fs_watch_stop"
    ) {
      return Promise.resolve(undefined);
    }
    return Promise.resolve(undefined);
  });
}

async function mountPanel(active = true) {
  const wrapper = mount(ResourceView, {
    props: { active },
    global: { directives: { tooltip: tooltipDirective } },
  });
  await flushPromises();
  // happy-dom 无真实布局：给文件区一个足够大的矩形，避免拖拽边界判定把测试坐标判为界外
  const list = wrapper.find(".resource-list").element as HTMLElement;
  Object.defineProperty(list, "getBoundingClientRect", {
    configurable: true,
    value: () => ({
      left: 0,
      top: 0,
      right: 2000,
      bottom: 2000,
      width: 2000,
      height: 2000,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }),
  });
  return wrapper;
}

async function openRowCtx(
  wrapper: VueWrapper,
  selector: string,
  index = 0,
) {
  await wrapper.findAll(selector)[index].trigger("contextmenu", {
    clientX: 200,
    clientY: 200,
  });
  await flushPromises();
}

async function clickCtxItem(wrapper: VueWrapper, label: string) {
  const btn = wrapper
    .findAll(".ctx-menu-item")
    .find((b) => b.text().trim() === label);
  expect(btn).toBeTruthy();
  await btn!.trigger("click");
  await flushPromises();
}

function fireWindowPointer(type: string, x: number, y: number) {
  window.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y }));
}

describe("ResourceView 文件树", () => {
  beforeEach(() => {
    store.server.startupWorkspace = rootPath;
    store.workspace = null;
    store.currentThreadWorkspace = null;
    store.newChatWorkspace = null;
    store.attachments = [];
    store.toast = "";
    clipboardFiles = [aTxt.path];
    createdFolder = null;
    createdTextFile = null;
    txtIconUri = "data:image/png;base64,TXTICON";
    mockedInvoke.mockClear();
    mockFs();
    __resetSessionFsForTest();
    __resetGitChangesForTest();
    __resetEditorTabsForTest();
    __resetSessionTabsForTest();
    // 默认存在一个活动会话标签：附件入口可见（隐藏场景由专门用例覆盖）
    tabs.push({
      id: "s1",
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
      loading: false,
      newChatWorkspace: null,
      interactions: [],
    });
    activeTabId.value = "s1";
    // 会话标签正在显示：附件入口可见（隐藏场景由专门用例覆盖）
    activeTabId.value = "s1";
  });

  afterEach(() => {
    unregisterComposerAddHandler("s1");
    __resetSessionFsForTest();
    vi.useRealTimers();
  });

  it("默认只展开第一层：根行 + 第一层子项，子目录收起", async () => {
    const wrapper = await mountPanel();
    expect(wrapper.find(".resource-root").text()).toContain("codex-ui");
    expect(wrapper.findAll(".resource-row.resource-dir")).toHaveLength(2);
    expect(wrapper.findAll(".resource-row.resource-file")).toHaveLength(1);
    expect(wrapper.text()).not.toContain("main.ts");
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_metadata", {
      workspace: rootPath,
      path: rootPath,
    });
    wrapper.unmount();
  });

  it("激活 diff 标签后切到资源面板：根未加载时先加载根并滚动定位到目标文件", async () => {
    const scrollIntoView = vi.fn();
    Object.defineProperty(Element.prototype, "scrollIntoView", {
      configurable: true,
      writable: true,
      value: scrollIntoView,
    });
    // 打开 diff 标签（Git 面板/消息入口），此时资源面板未激活、根尚未加载
    await openDiffTab({
      path: "src/main.ts",
      kind: "modify",
      diff: "diff",
      workspace: rootPath,
    });
    await flushPromises();
    expect(activeTabId.value).not.toBe("s1");

    // attachTo 挂到 document.body：revealAbsPath 的 document.querySelector 需要真实文档
    const wrapper = mount(ResourceView, {
      props: { active: true },
      attachTo: document.body,
      global: { directives: { tooltip: tooltipDirective } },
    });
    await flushPromises();
    await flushPromises();

    const row = wrapper
      .findAll(".resource-row")
      .find((r) => r.attributes("data-fs-path") === mainTs.path);
    expect(row?.exists()).toBe(true);
    expect(row?.classes()).toContain("active");
    expect(scrollIntoView).toHaveBeenCalled();
    wrapper.unmount();
  });

  it("点击文件行：联动 Git 面板高亮（revealGitFile）", async () => {
    const wrapper = await mountPanel();
    await wrapper.find(".resource-row.resource-file").trigger("click");
    await flushPromises();
    expect(gitRevealTarget.value).toMatchObject({
      workspace: rootPath,
      path: aTxt.path,
    });
    wrapper.unmount();
  });

  it("点击搜索结果文件：联动 Git 面板高亮（revealGitFile）", async () => {
    vi.useFakeTimers();
    const wrapper = await mountPanel();
    await wrapper.find(".history-search").setValue("main");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();

    await wrapper.find(".resource-result").trigger("click");
    await flushPromises();
    expect(gitRevealTarget.value).toMatchObject({
      workspace: rootPath,
      path: mainTs.path,
    });
    wrapper.unmount();
  });

  it("reveal 大小写不敏感：diff 路径与磁盘路径大小写不一致仍高亮定位", async () => {
    const scrollIntoView = vi.fn();
    Object.defineProperty(Element.prototype, "scrollIntoView", {
      configurable: true,
      writable: true,
      value: scrollIntoView,
    });
    const canonical = srcDir.path + "\\Main.TS";
    const base = mockedInvoke.getMockImplementation()!;
    mockedInvoke.mockImplementation((cmd, args) => {
      if (
        cmd === "session_fs_list" &&
        (args as { dir?: string }).dir === srcDir.path
      ) {
        return Promise.resolve([
          {
            ...mainTs,
            name: "Main.TS",
            path: canonical,
            relPath: "src/Main.TS",
          },
        ]);
      }
      return base(cmd, args);
    });

    await openDiffTab({
      path: "src/main.ts",
      kind: "modify",
      diff: "diff",
      workspace: rootPath,
    });
    await flushPromises();
    const wrapper = mount(ResourceView, {
      props: { active: true },
      attachTo: document.body,
      global: { directives: { tooltip: tooltipDirective } },
    });
    await flushPromises();
    await flushPromises();

    const row = wrapper
      .findAll(".resource-row")
      .find((r) => (r.attributes("data-fs-path") ?? "").toLowerCase() === canonical.toLowerCase());
    expect(row?.exists()).toBe(true);
    expect(row?.classes()).toContain("active");
    // selectedPath 被校正为树的规范路径（大小写以磁盘为准）
    expect(selectedPath.value).toBe(canonical);
    expect(scrollIntoView).toHaveBeenCalled();
    wrapper.unmount();
  });

  it("git 正斜杠 repoWorkspace 打开深层 diff：目录自动展开、文件选中并滚动定位", async () => {
    const scrollIntoView = vi.fn();
    Object.defineProperty(Element.prototype, "scrollIntoView", {
      configurable: true,
      writable: true,
      value: scrollIntoView,
    });
    const deepDir: FsEntry = {
      name: "deep",
      path: srcDir.path + "\\deep",
      relPath: "src/deep",
      isDir: true,
      size: null,
      modifiedAtMs: 0,
      createdAtMs: 0,
      childCount: 1,
    };
    const nestedDir: FsEntry = {
      name: "nested",
      path: deepDir.path + "\\nested",
      relPath: "src/deep/nested",
      isDir: true,
      size: null,
      modifiedAtMs: 0,
      createdAtMs: 0,
      childCount: 1,
    };
    const deepFile: FsEntry = {
      name: "file.txt",
      path: nestedDir.path + "\\file.txt",
      relPath: "src/deep/nested/file.txt",
      isDir: false,
      size: 1,
      modifiedAtMs: 0,
      createdAtMs: 0,
      childCount: null,
    };
    const base = mockedInvoke.getMockImplementation()!;
    mockedInvoke.mockImplementation((cmd, args) => {
      if (cmd === "session_fs_list") {
        const dir = (args as { dir?: string }).dir;
        if (dir === srcDir.path) return Promise.resolve([deepDir]);
        if (dir === deepDir.path) return Promise.resolve([nestedDir]);
        if (dir === nestedDir.path) return Promise.resolve([deepFile]);
      }
      return base(cmd, args);
    });

    // diff 标签 workspace 来自 git repo_workspace：正斜杠形式；EditorPane 会把它写入 store.workspace
    const fwdRoot = rootPath.replace(/\\/g, "/");
    store.workspace = fwdRoot;
    await openDiffTab({
      path: "src/deep/nested/file.txt",
      kind: "modify",
      diff: "diff",
      workspace: fwdRoot,
    });
    await flushPromises();
    const wrapper = mount(ResourceView, {
      props: { active: true },
      attachTo: document.body,
      global: { directives: { tooltip: tooltipDirective } },
    });
    await flushPromises();
    await flushPromises();

    const dirRows = wrapper.findAll(".resource-row.resource-dir");
    const dirPaths = dirRows.map((r) => r.attributes("data-fs-path"));
    // 根列表还含 node_modules 等其它目录，只断言目标祖先目录存在且已展开
    for (const p of [srcDir.path, deepDir.path, nestedDir.path]) {
      expect(dirPaths).toContain(p);
    }
    for (const r of dirRows.filter((r) =>
      [srcDir.path, deepDir.path, nestedDir.path].includes(
        r.attributes("data-fs-path") ?? "",
      ),
    )) {
      expect(r.classes()).not.toContain("collapsed");
    }
    const fileRow = wrapper
      .findAll(".resource-row.resource-file")
      .find((r) => r.attributes("data-fs-path") === deepFile.path);
    expect(fileRow?.exists()).toBe(true);
    expect(fileRow?.classes()).toContain("active");
    expect(scrollIntoView).toHaveBeenCalled();
    wrapper.unmount();
  });

  it("currentThreadWorkspace 为空字符串时回退启动工作目录", async () => {
    store.currentThreadWorkspace = "";
    const wrapper = await mountPanel();
    expect(wrapper.find(".resource-root").text()).toContain("codex-ui");
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_metadata", {
      workspace: rootPath,
      path: rootPath,
    });
    wrapper.unmount();
  });

  it("工作目录均未就绪时用 workspace_dir 兜底", async () => {
    store.currentThreadWorkspace = "";
    store.newChatWorkspace = "";
    store.server.startupWorkspace = "";
    const wrapper = await mountPanel();
    expect(wrapper.find(".resource-root").text()).toContain("codex-ui");
    expect(mockedInvoke).toHaveBeenCalledWith("startup_workspace");
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_metadata", {
      workspace: rootPath,
      path: rootPath,
    });
    wrapper.unmount();
  });

  it("点击目录行懒加载并展开子项，再点收起", async () => {
    const wrapper = await mountPanel();
    await wrapper.findAll(".resource-row.resource-dir")[0].trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("main.ts");
    expect(
      mockedInvoke.mock.calls.some(
        ([cmd, args]) =>
          cmd === "session_fs_list" &&
          (args as { dir?: string }).dir === srcDir.path,
      ),
    ).toBe(true);

    await wrapper.findAll(".resource-row.resource-dir")[0].trigger("click");
    await flushPromises();
    expect(wrapper.text()).not.toContain("main.ts");
    wrapper.unmount();
  });

  it("目录/根行带折叠箭头：根行展开为下箭头、子目录收起为右箭头；文件行无箭头", async () => {
    const wrapper = await mountPanel();
    expect(
      wrapper.find(".resource-root .resource-arrow path").attributes("d"),
    ).toBe("M20 12l-1.41-1.41L13 16.17V4h-2v12.17l-5.58-5.59L4 12l8 8 8-8z");
    expect(
      wrapper
        .findAll(".resource-row.resource-dir")[0]
        .find(".resource-arrow path")
        .attributes("d"),
    ).toBe("M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z");
    expect(
      wrapper.find(".resource-row.resource-file .resource-arrow").exists(),
    ).toBe(false);

    await wrapper.find(".resource-row.resource-root").trigger("click");
    await flushPromises();
    expect(
      wrapper.find(".resource-root .resource-arrow path").attributes("d"),
    ).toBe("M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z");
    wrapper.unmount();
  });

  it("搜索态结果行不带折叠箭头", async () => {
    vi.useFakeTimers();
    const wrapper = await mountPanel();
    await wrapper.find(".history-search").setValue("main");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();

    expect(wrapper.find(".resource-result .resource-arrow").exists()).toBe(
      false,
    );
    wrapper.unmount();
  });

  it("文件行渲染系统图标 img，目录/根行保留 SVG", async () => {
    const base = mockedInvoke.getMockImplementation()!;
    mockedInvoke.mockImplementation((cmd, args) => {
      if (cmd === "session_fs_icons") {
        const req = (args as { requests: { path: string }[] }).requests;
        return Promise.resolve(
          req.map((r) => ({ path: r.path, dataUri: "data:image/png;base64,ICON" })),
        );
      }
      return base(cmd, args);
    });
    const wrapper = await mountPanel();
    await flushPromises();

    // .txt 行复用「新建文本文件」菜单预取的系统 .txt 图标缓存
    const txtImg = wrapper.find(".resource-row.resource-file .resource-icon-img");
    expect(txtImg.exists()).toBe(true);
    expect(txtImg.attributes("src")).toBe("data:image/png;base64,TXTICON");

    // 展开 src 后其它文件走 session_fs_icons 批量取图
    await wrapper.findAll(".resource-row.resource-dir")[0].trigger("click");
    await flushPromises();
    const tsImg = wrapper
      .findAll(".resource-row.resource-file")[1]
      .find(".resource-icon-img");
    expect(tsImg.exists()).toBe(true);
    expect(tsImg.attributes("src")).toBe("data:image/png;base64,ICON");
    expect(
      wrapper.find(".resource-row.resource-dir .resource-icon svg").exists(),
    ).toBe(true);
    expect(
      wrapper.find(".resource-row.resource-root .resource-icon svg").exists(),
    ).toBe(true);
    wrapper.unmount();
  });

  it("文件右键菜单项与顺序", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-file");
    const labels = wrapper.findAll(".ctx-menu-item").map((b) => b.text().trim());
    expect(labels).toEqual([
      "打开",
      "复制",
      "属性",
      "删除",
      "重命名",
      "添加为会话附件",
      "在资源管理器中打开",
    ]);
    // “添加为会话附件”使用与行悬停按钮相同的 @ 图标
    expect(
      wrapper.findAll(".ctx-menu-item")[5].find("svg path")?.attributes("d"),
    ).toBe(ICON_AT);
    wrapper.unmount();
  });

  it("文本文件「打开」：在主窗口打开文件标签", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-file");
    await clickCtxItem(wrapper, "打开");
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_read", {
      workspace: rootPath,
      path: aTxt.path,
    });
    wrapper.unmount();
  });

  it("非文本文件右键菜单也显示“打开”", async () => {
    const wrapper = await mountPanel();
    await wrapper.findAll(".resource-row.resource-dir")[0].trigger("click");
    await flushPromises();
    const picRow = wrapper
      .findAll(".resource-row.resource-file")
      .find((w) => w.text().includes("pic.png"));
    expect(picRow).toBeTruthy();
    await picRow!.trigger("contextmenu", { clientX: 200, clientY: 200 });
    const labels = wrapper.findAll(".ctx-menu-item").map((b) => b.text().trim());
    expect(labels[0]).toBe("打开");
    wrapper.unmount();
  });

  it("已打开文件（编辑器标签）右键菜单不含「打开」", async () => {
    tabs.push({
      kind: TabKind.File,
      id: "f-open",
      workspace: rootPath,
      path: aTxt.path,
      title: "a.txt",
      icon: TabIcon.File,
      loading: false,
      error: "",
    } as unknown as FileEditorTab);
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-file");
    const labels = wrapper
      .findAll(".ctx-menu-item")
      .map((b) => b.text().trim());
    expect(labels).not.toContain("打开");
    expect(labels).toContain("复制");
    wrapper.unmount();
  });

  it("已打开文件（预览标签）右键菜单同样不含「打开」", async () => {
    tabs.push({
      kind: TabKind.Preview,
      previewType: "pdf",
      id: "p-open",
      workspace: rootPath,
      path: aTxt.path,
      title: "a.txt",
      icon: TabIcon.File,
      loading: false,
      error: "",
      imageUrl: "",
      pdfData: null,
      pageCount: null,
    } as unknown as PreviewEditorTab);
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-file");
    const labels = wrapper
      .findAll(".ctx-menu-item")
      .map((b) => b.text().trim());
    expect(labels).not.toContain("打开");
    wrapper.unmount();
  });

  it("根节点 tooltip 显示完整路径", async () => {
    const wrapper = mount(ResourceView, {
      props: { active: true },
      global: { directives: { tooltip: tooltipDirective } },
    });
    await flushPromises();
    expect(wrapper.find(".resource-root").attributes("data-tip")).toBe(
      rootPath,
    );
    wrapper.unmount();
  });

  it("单击文本文件行在主窗口打开文件标签", async () => {
    const wrapper = await mountPanel();
    await wrapper.find(".resource-row.resource-file").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_read", {
      workspace: rootPath,
      path: aTxt.path,
    });
    wrapper.unmount();
  });

  it("单击图像文件行：打开图像预览，不再提示无法打开", async () => {
    const wrapper = await mountPanel();
    await wrapper.findAll(".resource-row.resource-dir")[0].trigger("click");
    await flushPromises();
    const picRow = wrapper
      .findAll(".resource-row.resource-file")
      .find((w) => w.text().includes("pic.png"));
    expect(picRow).toBeTruthy();
    await picRow!.trigger("click");
    await flushPromises();
    expect(store.toast).not.toContain("该文件不是文本文件");
    expect(mockedConvertFileSrc).toHaveBeenCalledWith(picPng.path);
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_read",
      expect.anything(),
    );
    const previewTabs = tabs.filter((t) => t.kind === "preview");
    expect(previewTabs).toHaveLength(1);
    expect(previewTabs[0].previewType).toBe("image");
    wrapper.unmount();
  });

  it("右键 PDF 文件「打开」：进入 PDF 预览并读取二进制", async () => {
    const wrapper = await mountPanel();
    await wrapper.findAll(".resource-row.resource-dir")[0].trigger("click");
    await flushPromises();
    const pdfRow = wrapper
      .findAll(".resource-row.resource-file")
      .find((w) => w.text().includes("doc.pdf"));
    expect(pdfRow).toBeTruthy();
    await pdfRow!.trigger("contextmenu", { clientX: 200, clientY: 200 });
    await clickCtxItem(wrapper, "打开");

    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_read_bytes", {
      workspace: rootPath,
      path: docPdf.path,
    });
    const previewTabs = tabs.filter((t) => t.kind === "preview");
    expect(previewTabs).toHaveLength(1);
    expect(previewTabs[0].previewType).toBe("pdf");
    wrapper.unmount();
  });

  it("右键图像文件「打开」：进入图像预览，不探测文本", async () => {
    const wrapper = await mountPanel();
    await wrapper.findAll(".resource-row.resource-dir")[0].trigger("click");
    await flushPromises();
    const picRow = wrapper
      .findAll(".resource-row.resource-file")
      .find((w) => w.text().includes("pic.png"));
    expect(picRow).toBeTruthy();
    await picRow!.trigger("contextmenu", { clientX: 200, clientY: 200 });
    await clickCtxItem(wrapper, "打开");

    expect(mockedConvertFileSrc).toHaveBeenCalledWith(picPng.path);
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_probe_text",
      expect.anything(),
    );
    const previewTabs = tabs.filter((t) => t.kind === "preview");
    expect(previewTabs).toHaveLength(1);
    expect(previewTabs[0].previewType).toBe("image");
    wrapper.unmount();
  });

  it("探测失败：toast 错误且不触发预览", async () => {
    const wrapper = await mountPanel();
    const base = mockedInvoke.getMockImplementation()!;
    mockedInvoke.mockImplementation((cmd, args) => {
      if (cmd === "session_fs_probe_text") {
        return Promise.reject("probe error");
      }
      return base(cmd, args);
    });
    await wrapper.find(".resource-row.resource-file").trigger("click");
    await flushPromises();
    expect(store.toast).toContain("probe error");
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_read",
      expect.anything(),
    );
    wrapper.unmount();
  });

  it("目录右键菜单项与顺序", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-dir");
    const labels = wrapper.findAll(".ctx-menu-item").map((b) => b.text().trim());
    expect(labels).toEqual([
      "新建文本文件",
      "新建文件夹",
      "复制",
      "粘贴",
      "删除",
      "重命名",
      "添加为会话附件",
      "在此打开终端",
      "在资源管理器中打开",
    ]);
    wrapper.unmount();
  });

  it("根节点右键菜单不含新建会话，保留粘贴、在此打开终端与在资源管理器中打开", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-root");
    const labels = wrapper.findAll(".ctx-menu-item").map((b) => b.text().trim());
    expect(labels).toEqual([
      "新建文本文件",
      "新建文件夹",
      "粘贴",
      "在此打开终端",
      "在资源管理器中打开",
    ]);
    wrapper.unmount();
  });

  it("面板重新激活时按活动文件标签定位：展开祖先目录并选中文件", async () => {
    const wrapper = await mountPanel();
    // 初始仅展开第一层：src 目录收起，main.ts 不可见
    expect(wrapper.text()).not.toContain("main.ts");

    // 面板隐藏期间激活 src/main.ts 文件标签（EditorPane 的激活 watcher 同样会定位）
    await wrapper.setProps({ active: false });
    await openFileTab(rootPath, mainTs.path);
    await flushPromises();

    // 切回资源面板：按活动标签重新定位
    await wrapper.setProps({ active: true });
    await flushPromises();
    await flushPromises();

    expect(wrapper.text()).toContain("main.ts");
    expect(
      wrapper
        .findAll(".resource-row.resource-file")
        .find((r) => r.text().includes("main.ts"))
        ?.classes(),
    ).toContain("active");
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_list", {
      workspace: rootPath,
      dir: srcDir.path,
    });
    wrapper.unmount();
  });

  it("无活动会话标签时：隐藏「添加为会话附件」菜单项与行 @ 按钮", async () => {
    __resetSessionTabsForTest();
    const wrapper = await mountPanel();
    // 目录右键菜单不含附件项
    await openRowCtx(wrapper, ".resource-row.resource-dir");
    const labels = wrapper.findAll(".ctx-menu-item").map((b) => b.text().trim());
    expect(labels).not.toContain("添加为会话附件");
    // 文件右键菜单不含附件项
    await openRowCtx(wrapper, ".resource-row.resource-file");
    const fileLabels = wrapper
      .findAll(".ctx-menu-item")
      .map((b) => b.text().trim());
    expect(fileLabels).not.toContain("添加为会话附件");
    // 行悬停 @ 按钮隐藏
    expect(wrapper.find(".resource-add").exists()).toBe(false);
    wrapper.unmount();
  });

  it("新建文件夹：调用 session_fs_create_dir 并自动进入重命名", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-dir");
    await clickCtxItem(wrapper, "新建文件夹");
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_create_dir", {
      workspace: rootPath,
      dir: srcDir.path,
    });
    // 创建后展开/加载父目录并进入行内重命名
    await flushPromises();
    const input = wrapper.find(".resource-rename-input");
    expect(input.exists()).toBe(true);
    expect((input.element as HTMLInputElement).value).toBe("新建文件夹");
    wrapper.unmount();
  });

  it("新建文本文件：调用 session_fs_create_file 并自动进入重命名", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-dir");
    await clickCtxItem(wrapper, "新建文本文件");
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_create_file", {
      workspace: rootPath,
      dir: srcDir.path,
    });
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_create_dir",
      expect.anything(),
    );
    // 创建后展开/加载父目录并进入行内重命名
    await flushPromises();
    const input = wrapper.find(".resource-rename-input");
    expect(input.exists()).toBe(true);
    expect((input.element as HTMLInputElement).value).toBe("新建文本文件.txt");
    wrapper.unmount();
  });

  it("指针拖拽文件到目录：显示幽灵、高亮目标并调用 session_fs_move", async () => {
    const wrapper = await mountPanel();
    const fileRow = wrapper.find(".resource-row.resource-file");
    const dirRow = wrapper.findAll(".resource-row.resource-dir")[0];
    const hitSpy = vi
      .spyOn(document, "elementFromPoint")
      .mockReturnValue(dirRow.element as Element);
    await fileRow.trigger("pointerdown", {
      button: 0,
      clientX: 100,
      clientY: 100,
    });
    fireWindowPointer("pointermove", 120, 120);
    await flushPromises();
    expect(wrapper.find(".resource-drag-ghost").exists()).toBe(true);
    expect(dirRow.classes()).toContain("resource-drop-target");
    fireWindowPointer("pointerup", 120, 120);
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_move", {
      workspace: rootPath,
      src: aTxt.path,
      destDir: srcDir.path,
    });
    hitSpy.mockRestore();
    wrapper.unmount();
  });

  it("指针拖拽到自身：不高亮也不调用移动", async () => {
    const wrapper = await mountPanel();
    const dirRow = wrapper.findAll(".resource-row.resource-dir")[0];
    const hitSpy = vi
      .spyOn(document, "elementFromPoint")
      .mockReturnValue(dirRow.element as Element);
    await dirRow.trigger("pointerdown", {
      button: 0,
      clientX: 100,
      clientY: 100,
    });
    fireWindowPointer("pointermove", 120, 120);
    await flushPromises();
    expect(wrapper.find(".resource-drag-ghost").exists()).toBe(true);
    expect(dirRow.classes()).not.toContain("resource-drop-target");
    fireWindowPointer("pointerup", 120, 120);
    await flushPromises();
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_move",
      expect.anything(),
    );
    hitSpy.mockRestore();
    wrapper.unmount();
  });

  it("拖拽中按 Escape：取消并清除幽灵与高亮", async () => {
    const wrapper = await mountPanel();
    const fileRow = wrapper.find(".resource-row.resource-file");
    const dirRow = wrapper.findAll(".resource-row.resource-dir")[0];
    const hitSpy = vi
      .spyOn(document, "elementFromPoint")
      .mockReturnValue(dirRow.element as Element);
    await fileRow.trigger("pointerdown", {
      button: 0,
      clientX: 100,
      clientY: 100,
    });
    fireWindowPointer("pointermove", 120, 120);
    await flushPromises();
    expect(wrapper.find(".resource-drag-ghost").exists()).toBe(true);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flushPromises();
    expect(wrapper.find(".resource-drag-ghost").exists()).toBe(false);
    expect(dirRow.classes()).not.toContain("resource-drop-target");
    fireWindowPointer("pointerup", 120, 120);
    await flushPromises();
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_move",
      expect.anything(),
    );
    hitSpy.mockRestore();
    wrapper.unmount();
  });

  it("拖拽结束后的合成 click 被抑制，不触发文件打开", async () => {
    const wrapper = await mountPanel();
    const fileRow = wrapper.find(".resource-row.resource-file");
    const hitSpy = vi
      .spyOn(document, "elementFromPoint")
      .mockReturnValue(fileRow.element as Element);
    await fileRow.trigger("pointerdown", {
      button: 0,
      clientX: 100,
      clientY: 100,
    });
    fireWindowPointer("pointermove", 130, 130);
    fireWindowPointer("pointerup", 130, 130);
    await fileRow.trigger("click");
    await flushPromises();
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_probe_text",
      expect.anything(),
    );
    hitSpy.mockRestore();
    wrapper.unmount();
  });

  it("根节点「新建文本文件」菜单使用系统 .txt 图标 img", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-root");
    const item = wrapper
      .findAll(".ctx-menu-item")
      .find((b) => b.text().trim() === "新建文本文件")!;
    const img = item.find(".ctx-menu-item-img");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toBe(txtIconUri);
    expect(item.find("svg").exists()).toBe(false);
    wrapper.unmount();
  });

  it("目录「新建文本文件」菜单使用系统 .txt 图标 img", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-dir");
    const item = wrapper
      .findAll(".ctx-menu-item")
      .find((b) => b.text().trim() === "新建文本文件")!;
    const img = item.find(".ctx-menu-item-img");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toBe(txtIconUri);
    expect(item.find("svg").exists()).toBe(false);
    wrapper.unmount();
  });

  it("系统 .txt 图标不可用时「新建文本文件」回退加号 SVG", async () => {
    txtIconUri = null;
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-dir");
    const item = wrapper
      .findAll(".ctx-menu-item")
      .find((b) => b.text().trim() === "新建文本文件")!;
    expect(item.find(".ctx-menu-item-img").exists()).toBe(false);
    expect(item.find("svg").exists()).toBe(true);
    wrapper.unmount();
  });

  it("剪贴板无文件且无复制记录时右键菜单不显示「粘贴」", async () => {
    clipboardFiles = [];
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-dir");
    const labels = wrapper.findAll(".ctx-menu-item").map((b) => b.text().trim());
    expect(labels).not.toContain("粘贴");
    expect(labels[0]).toBe("新建文本文件");
    wrapper.unmount();
  });

  it("根节点「在此打开终端」：以工作根目录创建终端标签", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-root");
    await clickCtxItem(wrapper, "在此打开终端");
    const t = tabs.find((x) => x.kind === "terminal");
    expect(t).toBeTruthy();
    expect(t!.workspace).toBe(rootPath);
    wrapper.unmount();
  });

  it("目录「在此打开终端」：以该目录创建终端标签", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-dir");
    await clickCtxItem(wrapper, "在此打开终端");
    const t = tabs.find((x) => x.kind === "terminal");
    expect(t).toBeTruthy();
    expect(t!.workspace).toBe(srcDir.path);
    wrapper.unmount();
  });

  it("外部滚动不关闭菜单，面板内滚动关闭菜单", async () => {
    const wrapper = mount(ResourceView, {
      props: { active: true },
      attachTo: document.body,
    });
    await flushPromises();
    await openRowCtx(wrapper, ".resource-row.resource-file");
    expect(wrapper.find(".ctx-menu").exists()).toBe(true);

    // 模拟聊天区吸底滚动：非面板元素上的 scroll 事件不应关闭菜单
    const chatEl = document.createElement("div");
    chatEl.className = "chat-scroll";
    document.body.appendChild(chatEl);
    chatEl.dispatchEvent(new Event("scroll"));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".ctx-menu").exists()).toBe(true);
    chatEl.remove();

    // 面板列表自身滚动仍应关闭菜单
    wrapper.find(".resource-list").element.dispatchEvent(new Event("scroll"));
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
    wrapper.unmount();
  });

  it("重命名：内联输入回车调用 session_fs_rename", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-file");
    await clickCtxItem(wrapper, "重命名");
    const input = wrapper.find(".resource-rename-input");
    expect(input.exists()).toBe(true);
    await input.setValue("b.txt");
    await input.trigger("keydown.enter");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_rename", {
      workspace: rootPath,
      path: aTxt.path,
      newName: "b.txt",
    });
    wrapper.unmount();
  });

  it("删除：确认框提示后确认才调用 session_fs_delete", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-file");
    await clickCtxItem(wrapper, "删除");
    expect(wrapper.find(".modal-mask").exists()).toBe(true);
    expect(wrapper.text()).toContain("确定删除文件「a.txt」吗？此操作不可恢复。");
    expect(mockedInvoke).not.toHaveBeenCalledWith("session_fs_delete", expect.anything());

    const del = wrapper
      .findAll(".modal-foot .btn")
      .find((b) => b.text().trim() === "删除");
    await del!.trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_delete", {
      workspace: rootPath,
      path: aTxt.path,
    });
    expect(wrapper.find(".modal-mask").exists()).toBe(false);
    wrapper.unmount();
  });

  it("属性：打开对话框显示名称/类型/完整路径/大小", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-file");
    await clickCtxItem(wrapper, "属性");
    expect(wrapper.find(".modal-mask").exists()).toBe(true);
    const text = wrapper.find(".modal").text();
    expect(text).toContain("属性 - a.txt");
    expect(text).toContain("完整路径");
    expect(text).toContain(aTxt.path);
    expect(text).toContain("1.5 KB");
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_metadata", {
      workspace: rootPath,
      path: aTxt.path,
    });
    wrapper.unmount();
  });

  it("复制后目录上粘贴：内部剪贴板作为粘贴源", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-file");
    await clickCtxItem(wrapper, "复制");
    expect(store.toast).toContain("已复制");

    await openRowCtx(wrapper, ".resource-row.resource-dir");
    await clickCtxItem(wrapper, "粘贴");
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_paste", {
      workspace: rootPath,
      destDir: srcDir.path,
      sources: [aTxt.path],
    });
    wrapper.unmount();
  });

  it("添加为会话附件：路由到活动会话的注册处理器并携带协议附件", async () => {
    const addAttachment = vi.fn();
    registerComposerAddHandler("s1", addAttachment);
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-file");
    await clickCtxItem(wrapper, "添加为会话附件");
    expect(addAttachment).toHaveBeenCalledWith({
      type: "mention",
      name: "a.txt",
      path: "D:/codex/codex-ui/a.txt",
    });
    wrapper.unmount();
  });

  it("文件/目录行有 @ 添加附件按钮，根目录行没有", async () => {
    const wrapper = await mountPanel();
    expect(wrapper.find(".resource-root .resource-add").exists()).toBe(false);
    expect(
      wrapper.findAll(".resource-row.resource-dir .resource-add").length,
    ).toBeGreaterThan(0);
    expect(
      wrapper.findAll(".resource-row.resource-file .resource-add").length,
    ).toBeGreaterThan(0);
    // @ 按钮位于行首（首个元素子节点）
    expect(
      wrapper
        .find(".resource-row.resource-file")
        .element.firstElementChild?.classList.contains("resource-add"),
    ).toBe(true);
    expect(
      wrapper
        .findAll(".resource-row.resource-dir")[0]
        .element.firstElementChild?.classList.contains("resource-add"),
    ).toBe(true);
    // 与右键菜单共用同一个 @ 图标
    expect(
      wrapper
        .find(".resource-row.resource-file .resource-add svg path")
        .attributes("d"),
    ).toBe(ICON_AT);
    wrapper.unmount();
  });

  it("点击 @ 按钮路由到活动会话的注册处理器，且不触发行点击", async () => {
    const addAttachment = vi.fn();
    registerComposerAddHandler("s1", addAttachment);
    const wrapper = await mountPanel();

    // 文件行：只添加附件，不打开文件（不触发内容探测）
    await wrapper.find(".resource-row.resource-file .resource-add").trigger("click");
    expect(addAttachment).toHaveBeenCalledWith({
      type: "mention",
      name: "a.txt",
      path: "D:/codex/codex-ui/a.txt",
    });
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_probe_text",
      expect.anything(),
    );

    // 目录行：只添加附件，不切换展开状态
    const dirRow = wrapper.findAll(".resource-row.resource-dir")[0];
    const wasCollapsed = dirRow.classes().includes("collapsed");
    await dirRow.find(".resource-add").trigger("click");
    expect(dirRow.classes().includes("collapsed")).toBe(wasCollapsed);
    wrapper.unmount();
  });

  it("非会话视图（文件标签激活）时：隐藏「添加为会话附件」菜单项与行 @ 按钮", async () => {
    activeTabId.value = "file1";
    const wrapper = await mountPanel();
    // 树行与搜索结果行均无 @ 按钮
    expect(wrapper.find(".resource-add").exists()).toBe(false);
    // 文件右键菜单不含附件项
    await openRowCtx(wrapper, ".resource-row.resource-file");
    const fileLabels = wrapper
      .findAll(".ctx-menu-item")
      .map((b) => b.text().trim());
    expect(fileLabels).not.toContain("添加为会话附件");
    // 目录右键菜单不含附件项
    await openRowCtx(wrapper, ".resource-row.resource-dir");
    const dirLabels = wrapper
      .findAll(".ctx-menu-item")
      .map((b) => b.text().trim());
    expect(dirLabels).not.toContain("添加为会话附件");
    wrapper.unmount();
  });

  it("搜索结果行同样有 @ 添加附件按钮", async () => {
    vi.useFakeTimers();
    const wrapper = await mountPanel();
    await wrapper.find(".history-search").setValue("main");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();

    expect(wrapper.find(".resource-result .resource-add").exists()).toBe(true);
    // 搜索结果行同样为行首元素
    expect(
      wrapper
        .find(".resource-result")
        .element.firstElementChild?.classList.contains("resource-add"),
    ).toBe(true);
    expect(
      wrapper.find(".resource-result .resource-add svg path").attributes("d"),
    ).toBe(ICON_AT);
    wrapper.unmount();
  });

  it("搜索：防抖后调用 session_fs_search，结果显示，清除恢复树", async () => {
    vi.useFakeTimers();
    const wrapper = await mountPanel();
    await wrapper.find(".history-search").setValue("main");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();

    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_search", {
      workspace: rootPath,
      query: "main",
      limit: 200,
    });
    expect(wrapper.find(".resource-result").exists()).toBe(true);
    expect(wrapper.text()).toContain("src/main.ts");

    await wrapper.find(".history-search-clear").trigger("click");
    await flushPromises();
    expect(wrapper.find(".resource-result").exists()).toBe(false);
    expect(wrapper.find(".resource-root").exists()).toBe(true);
    wrapper.unmount();
  });

  it("点击搜索结果：回树定位（展开祖先、清除搜索）", async () => {
    vi.useFakeTimers();
    const wrapper = await mountPanel();
    await wrapper.find(".history-search").setValue("src");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();

    await wrapper.find(".resource-result").trigger("click");
    await flushPromises();
    expect(
      (wrapper.find(".history-search").element as HTMLInputElement).value,
    ).toBe("");
    expect(wrapper.find(".resource-result").exists()).toBe(false);
    expect(wrapper.text()).toContain("main.ts");
    expect(
      wrapper.find(".resource-row.resource-dir.active").exists(),
    ).toBe(true);
    wrapper.unmount();
  });

  it("搜索态单击文件结果打开预览", async () => {
    vi.useFakeTimers();
    const wrapper = await mountPanel();
    await wrapper.find(".history-search").setValue("main");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();

    await wrapper.find(".resource-result").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_read", {
      workspace: rootPath,
      path: mainTs.path,
    });
    wrapper.unmount();
  });

  it("搜索态单击 PDF 结果：进入 PDF 预览", async () => {
    vi.useFakeTimers();
    const wrapper = await mountPanel();
    await wrapper.find(".history-search").setValue("doc");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();

    await wrapper.find(".resource-result").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_read_bytes", {
      workspace: rootPath,
      path: docPdf.path,
    });
    wrapper.unmount();
  });
});

describe("ResourceView 工作区切换", () => {
  beforeEach(() => {
    store.server.startupWorkspace = rootPath;
    store.workspace = null;
    store.currentThreadWorkspace = null;
    store.newChatWorkspace = null;
    store.attachments = [];
    store.toast = "";
    mockedInvoke.mockClear();
    mockFs();
    __resetSessionFsForTest();
    __resetEditorTabsForTest();
    __resetSessionTabsForTest();
  });

  it("切换工作区保留旧树渲染，新数据到达后更新为新根", async () => {
    const wrapper = await mountPanel();
    expect(wrapper.text()).toContain("codex-ui");
    expect(wrapper.find(".resource-row").exists()).toBe(true);

    let resolveMeta!: (v: unknown) => void;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "session_fs_metadata") {
        return new Promise((r) => {
          resolveMeta = r;
        });
      }
      if (cmd === "session_fs_list") return Promise.resolve([]);
      if (
        cmd === "session_fs_watch_start" ||
        cmd === "session_fs_watch_stop"
      ) {
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });

    const other = "D:\\other";
    store.workspace = other;
    await flushPromises();
    // 新根 metadata 挂起中：旧树仍在渲染
    expect(wrapper.text()).toContain("codex-ui");
    expect(wrapper.find(".resource-row").exists()).toBe(true);

    resolveMeta({ ...rootEntry, name: "other", path: other });
    await flushPromises();
    await flushPromises();
    expect(wrapper.text()).toContain("other");
    wrapper.unmount();
  });
});
