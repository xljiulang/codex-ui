import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick, reactive } from "vue";
import type { EditorView } from "@codemirror/view";

vi.mock("../ChatView.vue", () => ({
  default: {
    name: "ChatViewStub",
    template: "<div class='chat-stub' />",
  },
}));

const { termFocus } = vi.hoisted(() => ({ termFocus: { calls: 0 } }));

vi.mock("@xterm/xterm", () => {
  class TerminalMock {
    loadAddon() {}
    open() {}
    onData() {}
    write() {}
    focus() {
      termFocus.calls++;
    }
    dispose() {}
  }
  return { Terminal: TerminalMock };
});
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit() {}
    proposeDimensions() {
      return { cols: 100, rows: 30 };
    }
  },
}));

// PDF 渲染较重且 happy-dom 无 canvas/worker：mock pdfjs-dist，验证标签装配即可
vi.mock("pdfjs-dist", () => ({
  GlobalWorkerOptions: {},
  getDocument: vi.fn(() => ({
    promise: Promise.reject(new Error("mocked pdf")),
    destroy: vi.fn(() => Promise.resolve()),
  })),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { invoke } from "@tauri-apps/api/core";
import EditorPane from "../EditorPane.vue";
import {
  __resetEditorTabsForTest,
  SETTINGS_TAB_ID,
  activateTab,
  activeTabId,
  closeTab,
  openCommitTab,
  openDiffTab,
  openFileTab,
  openSettingsTab,
  openPreviewTab,
  openTerminalTab,
  tabs,
  type TerminalEditorTab,
} from "../../composables/useEditorTabs";
import { tooltipDirective } from "../../directives/tooltip";
import {
  __resetSessionFsForTest,
  expanded,
  searchTerm,
  selectedPath,
} from "../../composables/useSessionFs";
import {
  __resetSessionTabsForTest,
  settleConfirm,
  store,
  type SessionTab,
} from "../../composables/useCodex";
import {
  ICON_GIT,
  ICON_SESSION_LOGO_C,
  ICON_TERMINAL,
} from "../../lib/icons";
import {
  __resetGitChangesForTest,
  gitRevealTarget,
} from "../../composables/useGitChanges";

const mockedInvoke = vi.mocked(invoke);
const root = "D:\\repo";
const aTxt = root + "\\a.txt";
const bTxt = root + "\\b.txt";
const aMd = root + "\\a.md";
const bMd = root + "\\b.md";

function fileContent(content: string, validUtf8 = true) {
  return {
    content,
    validUtf8,
    byteSize: content.length,
  };
}

/** 异步组件（CodeMirror/diff 懒加载 chunk）需要多等一拍才挂载 */
async function settle() {
  await flushPromises();
  await nextTick();
}

async function viewOf(
  wrapper: Awaited<ReturnType<typeof mount>>,
): Promise<EditorView> {
  return await vi.waitFor(
    () => {
      const host = wrapper.find(".text-editor-host");
      if (!host.exists()) throw new Error("waiting for editor host");
      const el = host.element as HTMLDivElement & { __cmView?: EditorView };
      if (!el.__cmView) throw new Error("waiting for editor view");
      return el.__cmView;
    },
    { timeout: 5000, interval: 20 },
  );
}

async function waitForEl(wrapper: Awaited<ReturnType<typeof mount>>, selector: string) {
  await vi.waitFor(
    () => {
      expect(wrapper.find(selector).exists()).toBe(true);
    },
    { timeout: 5000, interval: 20 },
  );
}

function mountPane() {
  return mount(EditorPane, {
    global: { directives: { tooltip: tooltipDirective } },
  });
}

describe("EditorPane 左侧多标签编辑区", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    termFocus.calls = 0;
    __resetEditorTabsForTest();
    __resetSessionFsForTest();
    __resetSessionTabsForTest();
    __resetGitChangesForTest();
    // 会话标签 fixture：多会话标签下 EditorPane 的会话标签来自 tabs
    tabs.push(reactive({
      id: "sess-1",
      kind: "chat",
      title: "新建会话",
      icon: "chat",
      threadId: "t1",
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      taskMode: "default",
      model: null,
      effort: null,
      plugins: { plugins: [], loaded: false },
      skills: { skills: [], loaded: false },
      creatingChat: false,
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
    } as SessionTab));
    activeTabId.value = "sess-1";
    store.workspace = null;
    store.confirm = null;
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("仅有会话标签时标签栏常驻显示，对话区直接可见", () => {
    const wrapper = mountPane();
    expect(wrapper.find(".editor-tabs").exists()).toBe(true);
    expect(wrapper.find(".chat-stub").exists()).toBe(true);
    wrapper.unmount();
  });

  it("零会话零文件标签时：空状态显示 Logo 与提示文案（无新建按钮），无活动标签「+」隐藏", () => {
    tabs.splice(0, tabs.length);
    activeTabId.value = "";
    const wrapper = mountPane();
    expect(wrapper.find(".editor-tabs").exists()).toBe(true);
    expect(wrapper.find(".editor-tab-add").exists()).toBe(false);
    expect(wrapper.find(".no-session-state").exists()).toBe(true);
    expect(wrapper.find(".no-session-state .empty-logo").exists()).toBe(true);
    expect(wrapper.find(".no-session-state").text()).toContain(
      "当前还没有任何打开的项",
    );
    expect(wrapper.find(".no-session-state .btn").exists()).toBe(false);
    wrapper.unmount();
  });

  it("仅有设置标签时：「+」隐藏（除设置外无任何标签）", () => {
    tabs.splice(0, tabs.length);
    tabs.push({
      id: "settings",
      kind: "settings" as const,
      title: "设置",
      icon: "settings" as const,
      workspace: null,
      loading: false,
    });
    activeTabId.value = "settings";
    const wrapper = mountPane();
    expect(wrapper.find(".editor-tab-add").exists()).toBe(false);
    wrapper.unmount();
  });

  it("切走再切回保持设置页状态", async () => {
    const wrapper = mountPane();
    openSettingsTab();
    await settle();
    const modelItem = wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("模型配置"))!;
    await modelItem.trigger("click");
    await settle();
    expect(
      (wrapper.find(".settings-section-model-config").element as HTMLElement)
        .style.display,
    ).not.toBe("none");

    activateTab("sess-1");
    await settle();
    activateTab(SETTINGS_TAB_ID);
    await settle();
    expect(
      (wrapper.find(".settings-section-model-config").element as HTMLElement)
        .style.display,
    ).not.toBe("none");
    wrapper.unmount();
  });

  it("关闭设置标签后重开重置", async () => {
    const wrapper = mountPane();
    openSettingsTab();
    await settle();
    const modelItem = wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("模型配置"))!;
    await modelItem.trigger("click");
    await settle();

    closeTab(SETTINGS_TAB_ID);
    await settle();
    expect(wrapper.find(".settings-nav-item").exists()).toBe(false);

    openSettingsTab();
    await settle();
    expect(wrapper.find(".settings-nav-item").exists()).toBe(true);
    expect(
      (wrapper.find(".settings-section-personalization").element as HTMLElement)
        .style.display,
    ).not.toBe("none");
    expect(
      (wrapper.find(".settings-section-model-config").element as HTMLElement)
        .style.display,
    ).toBe("none");
    wrapper.unmount();
  });

  it("关闭最后一个会话标签后：无活动标签，「+」隐藏", async () => {
    const wrapper = mountPane();
    expect(wrapper.find(".editor-tab-add").exists()).toBe(true);
    const tabEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("新建会话"))!;
    await tabEl.find(".editor-tab-close").trigger("click");
    await settle();
    expect(tabs.length).toBe(0);
    expect(wrapper.find(".editor-tab-add").exists()).toBe(false);
    expect(wrapper.find(".no-session-state").exists()).toBe(true);
    wrapper.unmount();
  });

  it("有活动标签时「+」可见，点击弹出「新建会话 / 新建终端」菜单（带图标）", async () => {
    tabs[0].workspace = "D:/repo";
    const wrapper = mountPane();
    expect(wrapper.find(".editor-tab-add").exists()).toBe(true);
    await wrapper.find(".editor-tab-add").trigger("click");
    const items = wrapper.findAll(".ctx-menu-item");
    expect(items.map((i) => i.text().trim())).toEqual([
      "新建会话",
      "新建终端",
    ]);
    // 新建会话：会话标签同款 Logo（六边形 + C 标记两条路径）
    const sessionPaths = items[0].findAll("svg path");
    expect(sessionPaths).toHaveLength(2);
    expect(sessionPaths[0].attributes("d")).toBe(
      "M12 2l8.66 5v10L12 22l-8.66-5V7z",
    );
    expect(sessionPaths[1].attributes("d")).toBe(ICON_SESSION_LOGO_C);
    expect(sessionPaths[1].classes()).toContain("logo-c");
    // 新建终端：仍为单路径图标
    expect(items[1].findAll("svg path")).toHaveLength(1);
    expect(items[1].find("svg path").attributes("d")).toBe(ICON_TERMINAL);
    wrapper.unmount();
  });

  it("点击「新建会话」：以活动标签工作区作为新会话目录", async () => {
    tabs[0].workspace = "D:/repo";
    const wrapper = mountPane();
    await wrapper.find(".editor-tab-add").trigger("click");
    await wrapper.findAll(".ctx-menu-item")[0].trigger("click");
    const added = tabs.find(
      (t): t is SessionTab =>
        t.kind === "chat" && t.threadId === null && t.id !== "sess-1",
    );
    expect(added).toBeTruthy();
    expect(added!.newChatWorkspace).toBe("D:/repo");
    wrapper.unmount();
  });

  it("点击「新建终端」：以活动标签工作区启动终端", async () => {
    tabs[0].workspace = "D:/repo";
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await wrapper.find(".editor-tab-add").trigger("click");
    await wrapper.findAll(".ctx-menu-item")[1].trigger("click");
    await settle();
    const spawn = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "terminal_spawn",
    );
    expect(spawn).toBeTruthy();
    expect((spawn![1] as { workspace?: string }).workspace).toBe("D:/repo");
    wrapper.unmount();
  });

  it("零会话但有文件标签时：标签栏显示、空状态不显示", async () => {
    tabs.splice(0, tabs.length);
    activeTabId.value = "";
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    expect(wrapper.find(".editor-tabs").exists()).toBe(true);
    expect(wrapper.find(".no-session-state").exists()).toBe(false);
    wrapper.unmount();
  });

  it("打开文件后显示标签栏：会话标签可关闭，文件标签在其后", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    expect(wrapper.find(".editor-tabs").exists()).toBe(true);
    const tabEls = wrapper.findAll(".editor-tab");
    expect(tabEls).toHaveLength(2);
    expect(tabEls[0].classes()).not.toContain("pinned");
    expect(tabEls[0].find(".editor-tab-logo").exists()).toBe(true);
    expect(tabEls[0].find(".editor-tab-label").exists()).toBe(true);
    expect(tabEls[0].text()).toContain("新建会话");
    expect(tabEls[0].find(".editor-tab-close").exists()).toBe(true);
    expect(tabEls[1].text()).toContain("a.txt");
    wrapper.unmount();
  });

  it("会话标签与文件标签同在滚动区内滚动", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();

    const track = wrapper.find(".editor-tabs-track");
    expect(track.exists()).toBe(true);
    expect(track.attributes("role")).toBe("tablist");
    expect(track.attributes("aria-label")).toBe("编辑标签");

    const tabEls = wrapper.findAll(".editor-tab");
    expect(tabEls).toHaveLength(2);
    expect(tabEls[0].classes()).not.toContain("pinned");
    expect(tabEls[0].find(".editor-tab-logo").exists()).toBe(true);

    const scrollerTabs = wrapper.find(".editor-tabs").findAll(".editor-tab");
    expect(scrollerTabs).toHaveLength(2);
    expect(scrollerTabs[0].find(".editor-tab-logo").exists()).toBe(true);
    expect(scrollerTabs[0].text()).toContain("新建会话");
    expect(scrollerTabs[1].text()).toContain("a.txt");
    expect(scrollerTabs[0].classes()).not.toContain("pinned");
    wrapper.unmount();
  });

  it("md 预览态在切换标签后保持：文件间切换与卸载重建均不丢失", async () => {
    mockedInvoke.mockImplementation((cmd, args) => {
      if (cmd === "session_fs_read") {
        const path = (args as { path?: string }).path;
        return Promise.resolve(
          path === bTxt ? fileContent("hello") : fileContent("# 标题"),
        );
      }
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, "a.md");
    await settle();
    await waitForEl(wrapper, ".text-editor-preview .md");
    // 默认渲染模式：无需切换即为预览态
    expect(
      wrapper.find(".text-editor-actions button[aria-label='编辑']").exists(),
    ).toBe(true);
    expect(wrapper.find(".text-editor-preview .md").exists()).toBe(true);
    const aTab = tabs.find((t) => t.title === "a.md")!;

    // 文件间切换（组件不卸载，仅 prop 变化）
    await openFileTab(root, bTxt);
    await settle();
    activateTab(aTab.id);
    await settle();
    expect(
      wrapper.find(".text-editor-actions button[aria-label='编辑']").exists(),
    ).toBe(true);
    expect(wrapper.find(".text-editor-preview .md").exists()).toBe(true);

    // 切到会话再回来（组件卸载重建）
    activeTabId.value = "sess-1";
    await settle();
    expect(wrapper.find(".text-editor-preview").exists()).toBe(false);
    activateTab(aTab.id);
    await settle();
    expect(
      wrapper.find(".text-editor-actions button[aria-label='编辑']").exists(),
    ).toBe(true);
    expect(wrapper.find(".text-editor-preview .md").exists()).toBe(true);
    wrapper.unmount();
  });

  it("a.md 预览后打开 b.md 并预览：显示 b 内容而非 a 内容", async () => {
    mockedInvoke.mockImplementation((cmd, args) => {
      if (cmd === "session_fs_read") {
        const path = (args as { path?: string }).path;
        return Promise.resolve(
          path === bMd
            ? fileContent("# B标题\n\nB 正文")
            : fileContent("# A标题\n\nA 正文"),
        );
      }
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();

    // 打开 a.md（默认渲染模式），确认渲染的是 A 内容
    await openFileTab(root, aMd);
    await settle();
    await vi.waitFor(
      () => {
        const txt = wrapper.find(".text-editor-preview .md").text();
        expect(txt).toContain("A标题");
      },
      { timeout: 5000, interval: 20 },
    );

    // 打开 b.md（默认渲染模式）：必须显示 B 内容，不得残留 A 内容
    await openFileTab(root, bMd);
    await settle();
    await vi.waitFor(
      () => {
        const txt = wrapper.find(".text-editor-preview .md").text();
        expect(txt).toContain("B标题");
        expect(txt).not.toContain("A标题");
      },
      { timeout: 5000, interval: 20 },
    );
    wrapper.unmount();
  });

  it("标签栏 tablist 角色由固定区包裹层承担", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();

    const scroller = wrapper.find(".editor-tabs");
    expect(scroller.attributes("role")).toBeUndefined();
    expect(scroller.attributes("aria-label")).toBeUndefined();
    expect(wrapper.find(".editor-tabs-track").attributes("role")).toBe("tablist");
    wrapper.unmount();
  });

  it("关闭最后一个文件标签后标签栏仍显示（会话标签常驻）", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    expect(wrapper.find(".editor-tabs").exists()).toBe(true);

    const fileTabEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("a.txt"))!;
    await fileTabEl.find(".editor-tab-close").trigger("click");
    await settle();
    expect(wrapper.find(".editor-tabs").exists()).toBe(true);
    expect(wrapper.find(".chat-stub").exists()).toBe(true);
    wrapper.unmount();
  });

  it("激活标签超出可视区时自动滚动到可视区", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    await openFileTab(root, bTxt);
    await settle();

    const scroller = wrapper.find(".editor-tabs").element as HTMLElement;
    const scrollTo = vi.fn();
    Object.defineProperty(scroller, "scrollTo", {
      value: scrollTo,
      configurable: true,
    });
    Object.defineProperty(scroller, "clientWidth", {
      value: 300,
      configurable: true,
    });
    Object.defineProperty(scroller, "scrollLeft", {
      value: 0,
      configurable: true,
    });
    const bTab = tabs.find((t) => t.title === "b.txt")!;
    const bEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("b.txt"))!
      .element as HTMLElement;
    Object.defineProperty(bEl, "offsetLeft", {
      value: 1200,
      configurable: true,
    });
    Object.defineProperty(bEl, "offsetWidth", {
      value: 120,
      configurable: true,
    });

    activeTabId.value = "sess-1";
    await settle();
    activateTab(bTab.id);
    await settle();
    await nextTick();

    expect(scrollTo).toHaveBeenCalledWith({
      left: 1028,
      behavior: "smooth",
    });
    wrapper.unmount();
  });

  it("标签溢出时显示左右箭头，点击按可视宽度翻页", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();

    const scroller = wrapper.find(".editor-tabs").element as HTMLElement;
    Object.defineProperty(scroller, "scrollWidth", {
      value: 1200,
      configurable: true,
    });
    Object.defineProperty(scroller, "clientWidth", {
      value: 300,
      configurable: true,
    });
    Object.defineProperty(scroller, "scrollLeft", {
      value: 0,
      configurable: true,
    });
    const scrollBy = vi.fn();
    Object.defineProperty(scroller, "scrollBy", {
      value: scrollBy,
      configurable: true,
    });

    scroller.dispatchEvent(new Event("scroll"));
    await nextTick();
    expect(wrapper.find(".editor-tab-scroll-right").exists()).toBe(true);
    expect(wrapper.find(".editor-tab-scroll-left").exists()).toBe(false);

    await wrapper.find(".editor-tab-scroll-right").trigger("click");
    expect(scrollBy).toHaveBeenCalledWith({
      left: 210,
      behavior: "smooth",
    });

    Object.defineProperty(scroller, "scrollLeft", {
      value: 900,
      configurable: true,
    });
    scroller.dispatchEvent(new Event("scroll"));
    await nextTick();
    expect(wrapper.find(".editor-tab-scroll-left").exists()).toBe(true);
    expect(wrapper.find(".editor-tab-scroll-right").exists()).toBe(false);

    await wrapper.find(".editor-tab-scroll-left").trigger("click");
    expect(scrollBy).toHaveBeenCalledWith({
      left: -210,
      behavior: "smooth",
    });
    wrapper.unmount();
  });

  it("会话标签右键菜单：与文件标签一致（关闭所有 + 关闭右边），关闭所有跳过未保存并提示", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    await openFileTab(root, bTxt);
    await settle();
    // 当前激活 b.txt：改成脏标记
    (await viewOf(wrapper)).dispatch({ changes: { from: 0, insert: "x" } });
    await flushPromises();
    expect(wrapper.findAll(".editor-tab-dirty")).toHaveLength(1);

    const chatTab = wrapper.findAll(".editor-tab")[0];
    await chatTab.trigger("contextmenu", { clientX: 100, clientY: 100 });
    const items = wrapper.findAll(".ctx-menu-item");
    expect(items.map((i) => i.text().trim())).toEqual([
      "关闭所有标签",
      "关闭右边所有标签",
    ]);

    await items[0].trigger("click");
    await flushPromises();
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
    expect(tabs.map((t) => t.title)).toEqual(["b.txt"]);
    expect(store.toast).toContain(
      "已跳过 1 个标签（未保存文件 / 运行中的终端 / 运行中的会话）",
    );
    wrapper.unmount();
  });

  it("非会话标签右键菜单：中间文件标签显示打开+关闭所有/左边/右边四项", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    await openFileTab(root, bTxt);
    await settle();
    await openFileTab(root, root + "\\c.txt");
    await settle();

    const bEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("b.txt"))!;
    await bEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    const items = wrapper.findAll(".ctx-menu-item");
    expect(items.map((i) => i.text().trim())).toEqual([
      "关闭所有标签",
      "关闭左边所有标签",
      "关闭右边所有标签",
      "在资源管理器中打开",
    ]);
    wrapper.unmount();
  });

  it("非会话标签右键菜单：首个文件标签左侧有会话标签（显示关闭左边），末个标签不显示关闭右边", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    await openFileTab(root, bTxt);
    await settle();
    await openFileTab(root, root + "\\c.txt");
    await settle();

    const aEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("a.txt"))!;
    await aEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    expect(
      wrapper.findAll(".ctx-menu-item").map((i) => i.text().trim()),
    ).toEqual([
      "关闭所有标签",
      "关闭左边所有标签",
      "关闭右边所有标签",
      "在资源管理器中打开",
    ]);

    const cEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("c.txt"))!;
    await cEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    expect(
      wrapper.findAll(".ctx-menu-item").map((i) => i.text().trim()),
    ).toEqual(["关闭所有标签", "关闭左边所有标签", "在资源管理器中打开"]);
    wrapper.unmount();
  });

  it("非会话标签右键「关闭所有标签」：关闭全部可关闭标签，跳过脏并提示", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    await openFileTab(root, bTxt);
    await settle();
    // 当前激活 b.txt：改成脏标记后从 b 的菜单关闭所有
    (await viewOf(wrapper)).dispatch({ changes: { from: 0, insert: "x" } });
    await flushPromises();
    expect(wrapper.findAll(".editor-tab-dirty")).toHaveLength(1);

    const bEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("b.txt"))!;
    await bEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    const items = wrapper.findAll(".ctx-menu-item");
    expect(items[0].text().trim()).toBe("关闭所有标签");
    await items[0].trigger("click");
    await flushPromises();
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
    expect(tabs.map((t) => t.title)).toEqual(["b.txt"]);
    expect(store.toast).toContain(
      "已跳过 1 个标签（未保存文件 / 运行中的终端 / 运行中的会话）",
    );
    wrapper.unmount();
  });

  it("非会话标签右键「关闭右边所有标签」：仅关右侧，保留目标与左侧（含会话标签）", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    await openFileTab(root, bTxt);
    await settle();
    await openFileTab(root, root + "\\c.txt");
    await settle();

    const aEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("a.txt"))!;
    await aEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    const items = wrapper.findAll(".ctx-menu-item");
    expect(items[2].text().trim()).toBe("关闭右边所有标签");
    await items[2].trigger("click");
    await flushPromises();
    expect(tabs.map((t) => t.title)).toEqual(["新建会话", "a.txt"]);
    wrapper.unmount();
  });

  it("文件标签右键「在资源管理器中打开」：reveal 文件绝对路径", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      if (cmd === "reveal_path") return Promise.resolve(undefined);
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    const aEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("a.txt"))!;
    await aEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    const items = wrapper.findAll(".ctx-menu-item");
    expect(items.map((i) => i.text().trim())).toEqual([
      "关闭所有标签",
      "关闭左边所有标签",
      "在资源管理器中打开",
    ]);
    await items[2].trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("reveal_path", { path: aTxt });
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
    wrapper.unmount();
  });

  it("外部文件标签右键「在资源管理器中打开」：按 root+文件名拼接", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      if (cmd === "reveal_path") return Promise.resolve(undefined);
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab("D:\\other", "x.txt");
    await settle();
    const xEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("x.txt"))!;
    await xEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    const items = wrapper.findAll(".ctx-menu-item");
    expect(items.map((i) => i.text().trim())).toEqual([
      "关闭所有标签",
      "关闭左边所有标签",
      "在资源管理器中打开",
    ]);
    await items[2].trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("reveal_path", {
      path: "D:\\other\\x.txt",
    });
    wrapper.unmount();
  });

  it("预览标签右键菜单含「在资源管理器中打开」并正确 reveal", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      if (cmd === "reveal_path") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openPreviewTab("image", root, "pic.png");
    await settle();
    const pEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("pic.png"))!;
    await pEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    const items = wrapper.findAll(".ctx-menu-item");
    expect(items.map((i) => i.text().trim())).toEqual([
      "关闭所有标签",
      "关闭左边所有标签",
      "在资源管理器中打开",
    ]);
    await items[2].trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("reveal_path", {
      path: root + "\\pic.png",
    });
    wrapper.unmount();
  });

  it("diff/终端标签右键菜单不含「在资源管理器中打开」", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "build_diff_preview") return Promise.resolve([]);
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openDiffTab({
      path: "a.txt",
      kind: "modify",
      diff: "diff --git a/a.txt b/a.txt\n@@ -1 +1 @@\n-x\n+y",
      workspace: root,
    });
    await settle();
    await openTerminalTab(root + "\\src");
    await settle();

    const diffEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("a.txt"))!;
    await diffEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    expect(
      wrapper.findAll(".ctx-menu-item").map((i) => i.text().trim()),
    ).not.toContain("在资源管理器中打开");

    const termEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("cmd"))!;
    await termEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    expect(
      wrapper.findAll(".ctx-menu-item").map((i) => i.text().trim()),
    ).not.toContain("在资源管理器中打开");
    wrapper.unmount();
  });

  it("终端标签右键菜单含「重命名」，文件/会话标签不含", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      if (cmd === "session_fs_read") return Promise.resolve(fileContent("x"));
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openTerminalTab(root + "\\src");
    await openFileTab(root, aTxt);
    await settle();

    const termEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("cmd"))!;
    await termEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    expect(
      wrapper.findAll(".ctx-menu-item").map((i) => i.text().trim()),
    ).toContain("重命名");

    const fileEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("a.txt"))!;
    await fileEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    expect(
      wrapper.findAll(".ctx-menu-item").map((i) => i.text().trim()),
    ).not.toContain("重命名");

    const sessEl = wrapper.findAll(".editor-tab")[0];
    await sessEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    expect(
      wrapper.findAll(".ctx-menu-item").map((i) => i.text().trim()),
    ).not.toContain("重命名");
    wrapper.unmount();
  });

  it("终端标签右键「重命名」：标题原位变输入框并预填当前标题", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openTerminalTab(root + "\\src");
    await settle();

    const termEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("cmd"))!;
    await termEl.trigger("contextmenu", { clientX: 100, clientY: 100 });
    const renameItem = wrapper
      .findAll(".ctx-menu-item")
      .find((i) => i.text().trim() === "重命名")!;
    await renameItem.trigger("click");
    await settle();

    const input = wrapper.find(".tab-rename-input");
    expect(input.exists()).toBe(true);
    expect((input.element as HTMLInputElement).value).toBe("终端 (cmd)");
    wrapper.unmount();
  });

  it("双击终端标签标题进入重命名；双击文件标签标题不进入", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      if (cmd === "session_fs_read") return Promise.resolve(fileContent("x"));
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openTerminalTab(root + "\\src");
    await openFileTab(root, aTxt);
    await settle();

    const termEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("cmd"))!;
    await termEl.find(".editor-tab-label").trigger("dblclick");
    expect(wrapper.find(".tab-rename-input").exists()).toBe(true);

    // 先退出终端重命名，避免输入框干扰文件标签断言
    await wrapper.find(".tab-rename-input").trigger("keydown.esc");
    expect(wrapper.find(".tab-rename-input").exists()).toBe(false);

    const fileEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("a.txt"))!;
    await fileEl.find(".editor-tab-label").trigger("dblclick");
    expect(wrapper.find(".tab-rename-input").exists()).toBe(false);
    wrapper.unmount();
  });

  it("输入新标题按 Enter：标签标题更新", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openTerminalTab(root + "\\src");
    await settle();

    const termEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("cmd"))!;
    await termEl.find(".editor-tab-label").trigger("dblclick");
    await wrapper.find(".tab-rename-input").setValue("构建终端");
    await wrapper.find(".tab-rename-input").trigger("keydown.enter");
    await settle();

    expect(wrapper.find(".tab-rename-input").exists()).toBe(false);
    expect(
      wrapper
        .findAll(".editor-tab")
        .some((w) => w.text().includes("构建终端")),
    ).toBe(true);
    wrapper.unmount();
  });

  it("失焦提交重命名：标签标题更新", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openTerminalTab(root + "\\src");
    await settle();

    const termEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("cmd"))!;
    await termEl.find(".editor-tab-label").trigger("dblclick");
    await wrapper.find(".tab-rename-input").setValue("失焦标题");
    await wrapper.find(".tab-rename-input").trigger("blur");
    await settle();

    expect(wrapper.find(".tab-rename-input").exists()).toBe(false);
    expect(
      wrapper
        .findAll(".editor-tab")
        .some((w) => w.text().includes("失焦标题")),
    ).toBe(true);
    wrapper.unmount();
  });

  it("输入全空白按 Enter：原标题不变", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openTerminalTab(root + "\\src");
    await settle();

    const termEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("cmd"))!;
    await termEl.find(".editor-tab-label").trigger("dblclick");
    await wrapper.find(".tab-rename-input").setValue("   ");
    await wrapper.find(".tab-rename-input").trigger("keydown.enter");
    await settle();

    expect(wrapper.find(".tab-rename-input").exists()).toBe(false);
    expect(
      wrapper
        .findAll(".editor-tab")
        .some((w) => w.text().includes("cmd")),
    ).toBe(true);
    wrapper.unmount();
  });

  it("按 Esc 取消重命名：标题不变、输入框关闭", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openTerminalTab(root + "\\src");
    await settle();

    const termEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("cmd"))!;
    await termEl.find(".editor-tab-label").trigger("dblclick");
    await wrapper.find(".tab-rename-input").setValue("临时标题");
    await wrapper.find(".tab-rename-input").trigger("keydown.esc");
    await settle();

    expect(wrapper.find(".tab-rename-input").exists()).toBe(false);
    expect(
      wrapper
        .findAll(".editor-tab")
        .some((w) => w.text().includes("cmd")),
    ).toBe(true);
    wrapper.unmount();
  });

  it("重命名输入框内右键不弹标签菜单", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openTerminalTab(root + "\\src");
    await settle();

    const termEl = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("cmd"))!;
    await termEl.find(".editor-tab-label").trigger("dblclick");
    await wrapper
      .find(".tab-rename-input")
      .trigger("contextmenu", { clientX: 100, clientY: 100 });
    expect(wrapper.findAll(".ctx-menu-item").length).toBe(0);
    wrapper.unmount();
  });

  it("激活 diff 标签：gitRevealTarget 指向该标签的 workspace/path", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "build_diff_preview") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openDiffTab({
      path: "a.txt",
      kind: "modify",
      diff: "diff --git a/a.txt b/a.txt\n@@ -1 +1 @@\n-x\n+y",
      workspace: root,
    });
    await settle();
    expect(gitRevealTarget.value).toMatchObject({
      workspace: root,
      path: "a.txt",
    });
    expect(typeof gitRevealTarget.value?.seq).toBe("number");
    wrapper.unmount();
  });

  it("激活文件标签：gitRevealTarget 指向该标签的 workspace/path", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") return Promise.resolve(fileContent("x"));
      if (cmd === "session_fs_metadata") {
        return Promise.resolve({
          name: "repo",
          path: root,
          relPath: ".",
          isDir: true,
          size: null,
          modifiedAtMs: 0,
          createdAtMs: 0,
          childCount: 1,
        });
      }
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      if (cmd === "session_fs_icon_for_ext") return Promise.resolve(null);
      if (cmd === "session_fs_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    expect(gitRevealTarget.value).toMatchObject({
      workspace: root,
      path: aTxt,
    });
    expect(typeof gitRevealTarget.value?.seq).toBe("number");
    wrapper.unmount();
  });

  it("激活预览标签：gitRevealTarget 指向该标签的 workspace/path", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read_bytes") {
        return Promise.resolve(new Uint8Array([104, 101, 108, 108, 111]).buffer);
      }
      if (cmd === "session_fs_metadata") {
        return Promise.resolve({
          name: "sub",
          path: root + "\\sub",
          relPath: ".",
          isDir: true,
          size: null,
          modifiedAtMs: 0,
          createdAtMs: 0,
          childCount: 0,
        });
      }
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      if (cmd === "session_fs_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openPreviewTab("pdf", root + "\\sub", "doc.pdf");
    await settle();
    expect(gitRevealTarget.value).toMatchObject({
      workspace: root + "\\sub",
      path: "doc.pdf",
    });
    expect(typeof gitRevealTarget.value?.seq).toBe("number");
    wrapper.unmount();
  });

  it("会话进行中：会话标签显示呼吸灯，结束后隐藏", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();

    const chatTab = () => wrapper.findAll(".editor-tab")[0];
    expect(chatTab().find(".editor-tab-run").exists()).toBe(false);

    (tabs[0] as SessionTab).turnActive = true;
    await nextTick();
    expect(chatTab().find(".editor-tab-run").exists()).toBe(true);

    (tabs[0] as SessionTab).turnActive = false;
    await nextTick();
    expect(chatTab().find(".editor-tab-run").exists()).toBe(false);
    wrapper.unmount();
  });

  it("打开文本文件：新增文件标签、渲染编辑器并激活", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    await settle();

    expect(wrapper.findAll(".editor-tab")).toHaveLength(2);
    const labels = wrapper.findAll(".editor-tab-label");
    expect(labels).toHaveLength(2);
    expect(labels[1].text()).toContain("a.txt");
    expect(wrapper.find(".editor-tab-icon").exists()).toBe(true);
    expect(wrapper.find(".editor-tab-icon svg").exists()).toBe(true);
    // 根目录文件：标题与相对路径相同，header 与标题均不显示 tooltip
    const fileTabEl = wrapper.findAll(".editor-tab")[1];
    expect(fileTabEl.attributes("data-tip")).toBeUndefined();
    expect(fileTabEl.find(".editor-tab-label").attributes("data-tip")).toBe("");
    await waitForEl(wrapper, ".text-editor-path");
    expect(wrapper.find(".text-editor-path").text()).toBe("a.txt");
    expect((await viewOf(wrapper)).state.doc.toString()).toBe("hello");
    expect(wrapper.find(".text-editor-dirty").exists()).toBe(false);
    wrapper.unmount();
  });

  it("子目录文件：标题与相对路径不同，标题上显示路径 tooltip", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, root + "\\src\\a.txt");
    await settle();
    await settle();

    const fileTabEl = wrapper.findAll(".editor-tab")[1];
    expect(fileTabEl.find(".editor-tab-label").text()).toBe("a.txt");
    expect(fileTabEl.attributes("data-tip")).toBeUndefined();
    expect(fileTabEl.find(".editor-tab-label").attributes("data-tip")).toBe(
      "src\\a.txt",
    );
    wrapper.unmount();
  });

  it("文件标签懒加载并显示文件系统图标", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      if (cmd === "session_fs_icons") {
        return Promise.resolve([
          { path: aTxt, dataUri: "data:image/png;base64,abc" },
        ]);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    await waitForEl(wrapper, ".editor-tab-icon-img");
    expect(wrapper.find(".editor-tab-icon-img").attributes("src")).toBe(
      "data:image/png;base64,abc",
    );
    wrapper.unmount();
  });

  it("编辑后出现未保存标记，保存按钮调用 session_fs_write 并复位", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("\uFEFFa\r\nb\r\n"));
      }
      if (cmd === "session_fs_write") {
        return Promise.resolve({});
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, "a.txt");
    await settle();

    const view = await viewOf(wrapper);
    view.dispatch({
      changes: { from: 0, insert: "x" },
      selection: { anchor: 1 },
    });
    await flushPromises();
    expect(wrapper.find(".editor-tab-dirty").exists()).toBe(true);
    expect(wrapper.find(".text-editor-dirty").exists()).toBe(true);

    await wrapper
      .find(".text-editor-actions .text-editor-icon-btn.primary")
      .trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_write", {
      workspace: root,
      path: "a.txt",
      content: "\uFEFFxa\r\nb\r\n",
    });
    expect(wrapper.find(".editor-tab-dirty").exists()).toBe(false);
    expect(wrapper.find(".text-editor-status-save").text()).toContain("已保存");
    wrapper.unmount();
  });

  it("非 UTF-8 文件：只读横幅提示且保存按钮禁用", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("abc", false));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, "a.txt");
    await settle();

    await waitForEl(wrapper, ".text-editor-banner");
    expect(wrapper.find(".text-editor-banner").exists()).toBe(true);
    const saveBtn = wrapper.find(
      ".text-editor-actions .text-editor-icon-btn.primary",
    );
    expect(saveBtn.attributes("disabled")).toBeDefined();
    wrapper.unmount();
  });

  it("切换文件标签保留未保存内容与脏标记", async () => {
    mockedInvoke.mockImplementation((cmd, args) => {
      if (cmd === "session_fs_read") {
        const path = (args as { path: string }).path;
        return Promise.resolve(fileContent(path === bTxt ? "hello2" : "hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    const view = await viewOf(wrapper);
    view.dispatch({ changes: { from: 0, insert: "x" } });
    await flushPromises();

    await openFileTab(root, bTxt);
    await settle();
    expect((await viewOf(wrapper)).state.doc.toString()).toBe("hello2");

    const aTab = tabs.find((t) => t.title === "a.txt");
    expect(aTab).toBeTruthy();
    activateTab(aTab!.id);
    await settle();
    expect((await viewOf(wrapper)).state.doc.toString()).toBe("xhello");
    expect(wrapper.find(".editor-tab-dirty").exists()).toBe(true);
    wrapper.unmount();
  });

  it("切到对话主标签后编辑器卸载，切回文件标签恢复内容", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, "a.txt");
    await settle();
    const view = await viewOf(wrapper);
    view.dispatch({ changes: { from: 0, insert: "x" } });
    await flushPromises();

    activeTabId.value = "sess-1";
    await settle();
    expect(wrapper.find(".text-editor-host").exists()).toBe(false);
    expect(wrapper.find(".chat-stub").exists()).toBe(true);

    const aTab = tabs.find((t) => t.title === "a.txt")!;
    activateTab(aTab.id);
    await settle();
    expect((await viewOf(wrapper)).state.doc.toString()).toBe("xhello");
    wrapper.unmount();
  });

  it("关闭脏文件标签弹出确认：取消保留、放弃关闭、保存并关闭写盘", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      if (cmd === "session_fs_write") {
        return Promise.resolve({});
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, "a.txt");
    await settle();
    const view = await viewOf(wrapper);
    view.dispatch({ changes: { from: 0, insert: "x" } });
    await flushPromises();
    const aTab = tabs.find((t) => t.title === "a.txt")!;

    closeTab(aTab.id);
    await flushPromises();
    expect(wrapper.find(".text-editor-overlay").exists()).toBe(true);
    expect(wrapper.find(".text-editor-confirm-msg").text()).toContain("a.txt");

    // 取消：保留标签
    const cancelBtn = wrapper
      .findAll(".text-editor-confirm-actions .text-editor-btn")
      .find((b) => b.text() === "取消")!;
    await cancelBtn.trigger("click");
    expect(wrapper.find(".text-editor-overlay").exists()).toBe(false);
    expect(tabs.some((t) => t.id === aTab.id)).toBe(true);

    // 放弃并关闭：移除标签、不写盘
    closeTab(aTab.id);
    await flushPromises();
    const discardBtn = wrapper
      .findAll(".text-editor-confirm-actions .text-editor-btn")
      .find((b) => b.text() === "放弃并关闭")!;
    await discardBtn.trigger("click");
    await flushPromises();
    expect(tabs.some((t) => t.id === aTab.id)).toBe(false);
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "session_fs_write",
      expect.anything(),
    );
    wrapper.unmount();
  });

  it("保存并关闭：写盘后移除标签", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      if (cmd === "session_fs_write") {
        return Promise.resolve({});
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, "a.txt");
    await settle();
    (await viewOf(wrapper)).dispatch({ changes: { from: 0, insert: "x" } });
    await flushPromises();
    const aTab = tabs.find((t) => t.title === "a.txt")!;

    closeTab(aTab.id);
    await flushPromises();
    const saveBtn = wrapper
      .findAll(".text-editor-confirm-actions .text-editor-btn")
      .find((b) => b.text() === "保存并关闭")!;
    await saveBtn.trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_write", {
      workspace: root,
      path: "a.txt",
      content: "xhello",
    });
    expect(tabs.some((t) => t.id === aTab.id)).toBe(false);
    expect(activeTabId.value).toBe("sess-1");
    wrapper.unmount();
  });

  it("打开 diff 标签：展示类型徽标与内联行", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "build_diff_preview") {
        return Promise.resolve([
          { kind: "add", newNo: 1, text: "hello" },
        ]);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openDiffTab({
      path: "a.txt",
      kind: "add",
      diff: "diff --git a/a.txt b/a.txt\n@@ -0,0 +1 @@\n+hello",
      workspace: root,
    });
    await settle();
    await waitForEl(wrapper, ".diff-window-path");

    expect(wrapper.find(".editor-tab-kind").text()).toBe("新增");
    expect(wrapper.find(".diff-window-path").text()).toBe("a.txt");
    const titleSpans = wrapper.findAll(".diff-window-title > span");
    expect(titleSpans[0].classes()).toContain("diff-window-path");
    expect(titleSpans[1].classes()).toContain("change-kind");
    expect(wrapper.find(".diff-row.add").exists()).toBe(true);
    expect(wrapper.find(".diff-row.add .diff-text").text()).toBe("hello");
    wrapper.unmount();
  });

  it("打开图像预览：标签栏显示「预览」徽标并渲染预览区", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openPreviewTab("image", root, "pic.png");
    await settle();

    expect(wrapper.find(".editor-tabs").exists()).toBe(true);
    const tabEls = wrapper.findAll(".editor-tab");
    expect(tabEls).toHaveLength(2);
    expect(tabEls[1].find(".editor-tab-label").text()).toBe("pic.png");
    expect(tabEls[1].find(".editor-tab-kind").text()).toBe("预览");
    // 根目录文件：标题与相对路径相同，header 与标题均不显示 tooltip
    expect(tabEls[1].attributes("data-tip")).toBeUndefined();
    expect(tabEls[1].find(".editor-tab-label").attributes("data-tip")).toBe("");
    await waitForEl(wrapper, ".preview-pane");
    wrapper.unmount();
  });

  it("打开提交详情标签：渲染元信息与文件列表，点击文件打开该提交的 diff 标签", async () => {
    const hash = "d".repeat(40);
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "git_changes_commit_detail") {
        return Promise.resolve({
          hash,
          shortHash: "ddddddd",
          subject: "feat: commit",
          body: "feat: commit\n\nbody text",
          author: "tester",
          authorEmail: "t@t",
          authorTimeSecs: 1700000000,
          committer: "tester",
          committerEmail: "t@t",
          committerTimeSecs: 1700000000,
          parents: [],
          files: [
            {
              path: "src/a.txt",
              status: "modified",
              insertions: 2,
              deletions: 1,
              binary: false,
            },
            {
              path: "bin.dat",
              status: "added",
              insertions: 0,
              deletions: 0,
              binary: true,
            },
          ],
        });
      }
      if (cmd === "git_changes_commit_file_diff") {
        return Promise.resolve([
          { kind: "ctx", oldNo: 1, newNo: 1, text: "a" },
          { kind: "del", oldNo: 2, text: "x" },
          { kind: "add", newNo: 2, text: "y" },
        ]);
      }
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });

    const wrapper = mountPane();
    await openCommitTab(root, hash, "feat: commit");
    await settle();
    await waitForEl(wrapper, ".commit-pane");

    // 提交标签图标与 GitView 面板 tab 同款
    expect(wrapper.find(".editor-tab-icon svg path").attributes("d")).toBe(
      ICON_GIT,
    );
    expect(wrapper.find(".commit-subject").text()).toBe("feat: commit");
    expect(wrapper.find(".commit-hash").text()).toBe("ddddddd");
    expect(wrapper.find(".commit-meta").text()).toContain("tester");
    expect(wrapper.find(".commit-body").text()).toContain("body text");
    const fileRows = wrapper.findAll(".commit-file");
    expect(fileRows).toHaveLength(2);
    expect(fileRows[0].find(".commit-file-path").text()).toBe("src/a.txt");
    expect(fileRows[0].find(".commit-file-stats").text()).toContain("+2");
    expect(fileRows[0].find(".commit-file-stats").text()).toContain("-1");
    expect(fileRows[1].find(".commit-file-stats").text()).toBe("二进制");

    // 点击文件行：调用该提交的文件 diff 并打开 Diff 标签
    await fileRows[0].trigger("click");
    await settle();
    const diffCall = mockedInvoke.mock.calls.find(
      ([cmd]) => cmd === "git_changes_commit_file_diff",
    );
    expect(diffCall).toBeTruthy();
    expect(diffCall?.[1]).toEqual({
      workspace: root,
      hash,
      path: "src/a.txt",
    });
    const diffTab = tabs.find((t) => t.kind === "diff");
    expect(diffTab).toBeTruthy();
    expect(diffTab?.path).toBe("src/a.txt");
    await waitForEl(wrapper, ".diff-window-path");
    expect(wrapper.find(".diff-window-path").text()).toBe("src/a.txt");
    wrapper.unmount();
  });

  it("打开 PDF 预览标签：读取二进制并渲染预览区", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read_bytes") {
        return Promise.resolve(new Uint8Array([37, 80, 68, 70, 45, 49]).buffer);
      }
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openPreviewTab("pdf", root, "doc.pdf");
    await settle();

    const tabEls = wrapper.findAll(".editor-tab");
    expect(tabEls).toHaveLength(2);
    expect(tabEls[1].find(".editor-tab-label").text()).toBe("doc.pdf");
    expect(tabEls[1].find(".editor-tab-kind").text()).toBe("预览");
    await waitForEl(wrapper, ".preview-pane");
    wrapper.unmount();
  });

  it("打开终端标签：显示终端图标标签并渲染终端面板", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openTerminalTab(root + "\\src");
    await settle();

    expect(wrapper.find(".editor-tabs").exists()).toBe(true);
    const tabEls = wrapper.findAll(".editor-tab");
    expect(tabEls).toHaveLength(2);
    expect(tabEls[1].find(".editor-tab-label").text()).toBe("终端 (cmd)");
    // 终端标签标题固定为「终端 (cmd)」，无 ToolTip（header 与 label 均无 data-tip）
    expect(tabEls[1].attributes("data-tip")).toBeUndefined();
    expect(tabEls[1].find(".editor-tab-label").attributes("data-tip")).toBe("");
    expect(tabEls[1].find(".editor-tab-icon svg").exists()).toBe(true);
    await waitForEl(wrapper, ".terminal-pane");
    wrapper.unmount();
  });

  it("切换回已打开的终端标签时聚焦 xterm", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openTerminalTab(root + "\\src");
    await settle();
    await waitForEl(wrapper, ".terminal-pane");
    expect(termFocus.calls).toBeGreaterThanOrEqual(1);

    // 切到会话标签，再点回终端标签：激活路径应再次聚焦
    activeTabId.value = "sess-1";
    await settle();
    const before = termFocus.calls;
    const termTab = wrapper
      .findAll(".editor-tab")
      .find((w) => w.text().includes("cmd"))!;
    await termTab.trigger("click");
    await settle();
    expect(termFocus.calls).toBeGreaterThan(before);
    wrapper.unmount();
  });

  it("点击关闭按钮关闭运行中的终端：弹确认，取消保留、确认终止并关闭", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openTerminalTab(root + "\\src");
    await settle();
    await waitForEl(wrapper, ".terminal-pane");
    const t = tabs.find(
      (x): x is TerminalEditorTab => x.kind === "terminal",
    )!;
    t.busy = true;
    await nextTick();

    const termEl = () => wrapper.findAll(".editor-tab")[1];
    await termEl().find(".editor-tab-close").trigger("click");
    await nextTick();
    expect(store.confirm?.title).toBe("关闭终端");

    // 取消：标签保留、进程不终止
    settleConfirm(false);
    await flushPromises();
    expect(tabs.some((x) => x.id === t.id)).toBe(true);

    // 再次关闭并确认：终止进程并移除标签
    await termEl().find(".editor-tab-close").trigger("click");
    await nextTick();
    expect(store.confirm).not.toBeNull();
    settleConfirm(true);
    await flushPromises();
    expect(tabs.some((x) => x.id === t.id)).toBe(false);
    expect(mockedInvoke).toHaveBeenCalledWith("terminal_kill", { id: t.id });
    wrapper.unmount();
  });

  it("终端标签：命令执行中显示呼吸圆点，空闲隐藏且多终端独立", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openTerminalTab(root + "\\src");
    await settle();
    await waitForEl(wrapper, ".terminal-pane");
    await openTerminalTab(root + "\\lib");
    await settle();

    // 终端标签标题固定为 cmd：按标签位置（chat 固定第 0 位）与 cwd 区分
    const termTabs = wrapper.findAll(".editor-tab");
    const t1 = tabs.find(
      (x): x is TerminalEditorTab =>
        x.kind === "terminal" && x.workspace === root + "\\src",
    )!;
    const t2 = tabs.find(
      (x): x is TerminalEditorTab =>
        x.kind === "terminal" && x.workspace === root + "\\lib",
    )!;

    expect(termTabs[1].find(".editor-tab-run").exists()).toBe(false);

    t1.busy = true;
    await nextTick();
    expect(termTabs[1].find(".editor-tab-run.inline").exists()).toBe(true);
    expect(termTabs[2].find(".editor-tab-run").exists()).toBe(false);

    t2.busy = true;
    t1.busy = false;
    await nextTick();
    expect(termTabs[1].find(".editor-tab-run").exists()).toBe(false);
    expect(termTabs[2].find(".editor-tab-run.inline").exists()).toBe(true);
    wrapper.unmount();
  });

  it("终端标签：已退出或启动失败时即使 busy 也不显示呼吸圆点", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openTerminalTab(root + "\\src");
    await settle();
    await waitForEl(wrapper, ".terminal-pane");

    const termEl = () => wrapper.findAll(".editor-tab")[1];
    const t = tabs.find(
      (x): x is TerminalEditorTab => x.kind === "terminal",
    )!;
    t.busy = true;
    await nextTick();
    expect(termEl().find(".editor-tab-run").exists()).toBe(true);

    t.exited = true;
    await nextTick();
    expect(termEl().find(".editor-tab-run").exists()).toBe(false);

    t.exited = false;
    t.error = "spawn boom";
    await nextTick();
    expect(termEl().find(".editor-tab-run").exists()).toBe(false);
    wrapper.unmount();
  });

  it("激活文件标签：资源树同步选中并展开所在目录", async () => {
    store.workspace = root;
    const mainTs = root + "\\src\\main.ts";
    mockedInvoke.mockImplementation((cmd, args) => {
      if (cmd === "session_fs_read") return Promise.resolve(fileContent("x"));
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      if (cmd === "session_fs_icon_for_ext") return Promise.resolve(null);
      if (cmd === "session_fs_list") {
        const dir = (args as { dir?: string }).dir;
        if (dir === root + "\\src") {
          return Promise.resolve([
            {
              name: "main.ts",
              path: mainTs,
              relPath: "src/main.ts",
              isDir: false,
              size: 1,
              modifiedAtMs: 0,
              createdAtMs: 0,
              childCount: null,
            },
          ]);
        }
        return Promise.resolve([]);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, mainTs);
    await settle();
    expect(selectedPath.value).toBe(mainTs);
    expect(expanded.has(root + "\\src")).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_list", {
      workspace: root,
      dir: root + "\\src",
    });
    wrapper.unmount();
  });

  it("激活 Diff 标签：同样同步资源树定位", async () => {
    store.workspace = root;
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "build_diff_preview") return Promise.resolve([]);
      if (cmd === "session_fs_metadata") {
        return Promise.resolve({
          name: "repo",
          path: root,
          relPath: ".",
          isDir: true,
          size: null,
          modifiedAtMs: 0,
          createdAtMs: 0,
          childCount: 1,
        });
      }
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      if (cmd === "session_fs_icon_for_ext") return Promise.resolve(null);
      if (cmd === "session_fs_list") return Promise.resolve([]);
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openDiffTab({
      path: "src/main.ts",
      kind: "modify",
      diff: "diff",
      workspace: root,
    });
    await settle();
    expect(selectedPath.value).toBe(root + "\\src\\main.ts");
    expect(expanded.has(root + "\\src")).toBe(true);
    // 根未加载（资源面板从未激活）时由定位流程补齐加载，联动才不失效
    expect(expanded.has(root)).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_metadata", {
      workspace: root,
      path: root,
    });
    wrapper.unmount();
  });

  it("工作区外文件标签激活：资源树跟随该文件工作区并选中", async () => {
    store.workspace = root;
    const outside = "D:\\outside";
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") return Promise.resolve(fileContent("x"));
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      if (cmd === "session_fs_icon_for_ext") return Promise.resolve(null);
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(outside, "f.txt");
    await settle();
    expect(store.workspace).toBe(outside);
    expect(selectedPath.value).toBe(outside + "\\f.txt");
    wrapper.unmount();
  });

  it("激活文件标签：store.workspace 跟随其工作区；切回会话标签复位 null", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();
    expect(store.workspace).toBe(root);

    activeTabId.value = "sess-1";
    await settle();
    expect(store.workspace).toBeNull();
    wrapper.unmount();
  });

  it("激活 diff/预览/终端标签：store.workspace 分别跟随其工作区", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "build_diff_preview") return Promise.resolve([]);
      if (cmd === "session_fs_read_bytes") {
        return Promise.resolve(new Uint8Array([104, 101, 108, 108, 111]).buffer);
      }
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      if (cmd === "terminal_spawn") return Promise.resolve({});
      if (cmd === "terminal_resize") return Promise.resolve(undefined);
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openDiffTab({
      path: "a.txt",
      kind: "modify",
      diff: "diff",
      workspace: root,
    });
    await settle();
    expect(store.workspace).toBe(root);

    await openPreviewTab("pdf", root + "\\sub", "doc.pdf");
    await settle();
    expect(store.workspace).toBe(root + "\\sub");

    await openTerminalTab(root + "\\term");
    await settle();
    expect(store.workspace).toBe(root + "\\term");
    wrapper.unmount();
  });

  it("搜索态下激活文件标签：退出搜索并定位到树", async () => {
    store.workspace = root;
    searchTerm.value = "main";
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") return Promise.resolve(fileContent("x"));
      if (cmd === "session_fs_icons") return Promise.resolve([]);
      if (cmd === "session_fs_icon_for_ext") return Promise.resolve(null);
      if (cmd === "session_fs_list") return Promise.resolve([]);
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mountPane();
    await openFileTab(root, root + "\\a.txt");
    await settle();
    expect(searchTerm.value).toBe("");
    expect(selectedPath.value).toBe(root + "\\a.txt");
    wrapper.unmount();
  });

  it("统一列表三块排序：会话标签在前、终端居中、文件按打开顺序在后", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      if (cmd === "terminal_spawn") return Promise.resolve({});
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openTerminalTab(root);
    await openFileTab(root, aTxt);
    await openFileTab(root, bTxt);
    await settle();

    // 会话 fixture（sess-1）恒在前，终端居中，文件按打开顺序在后
    const labels = wrapper.findAll(".editor-tab").map((w) => w.text().trim());
    expect(labels[0]).toContain("新建会话");
    expect(labels[1]).toBe("终端 (cmd)");
    expect(labels[2]).toBe("a.txt");
    expect(labels[3]).toBe("b.txt");
    wrapper.unmount();
  });

  it("多会话时会话标签右键菜单：按整体顺序显示关闭左边/右边", async () => {
    tabs.push({
      id: "sess-2",
      kind: "chat",
      title: "会话二",
      icon: "chat",
      threadId: "t2",
      name: "",
      nameIsFirstMessage: false,
      permissionMode: "ask-for-approval",
      taskMode: "default",
      model: null,
      effort: null,
      plugins: { plugins: [], loaded: false },
      skills: { skills: [], loaded: false },
      creatingChat: false,
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
    });
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("x"));
      }
      if (cmd === "session_fs_icons") {
        return Promise.resolve([]);
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountPane();
    await openFileTab(root, aTxt);
    await settle();

    // 顺序：sess-1, sess-2, a.txt → 首个会话只有右边；末个会话左右都有
    const firstChat = wrapper.findAll(".editor-tab")[0];
    await firstChat.trigger("contextmenu", { clientX: 100, clientY: 100 });
    expect(wrapper.findAll(".ctx-menu-item").map((i) => i.text().trim())).toEqual(
      ["关闭所有标签", "关闭右边所有标签"],
    );
    await wrapper.find(".ctx-menu").trigger("click");

    const secondChat = wrapper.findAll(".editor-tab")[1];
    await secondChat.trigger("contextmenu", { clientX: 100, clientY: 100 });
    expect(wrapper.findAll(".ctx-menu-item").map((i) => i.text().trim())).toEqual(
      ["关闭所有标签", "关闭左边所有标签", "关闭右边所有标签"],
    );
    wrapper.unmount();
  });
});
