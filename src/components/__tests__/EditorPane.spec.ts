import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick } from "vue";
import type { EditorView } from "@codemirror/view";

vi.mock("../ChatView.vue", () => ({
  default: {
    name: "ChatViewStub",
    template: "<div class='chat-stub' />",
  },
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { invoke } from "@tauri-apps/api/core";
import EditorPane from "../EditorPane.vue";
import {
  __resetEditorTabsForTest,
  activateTab,
  activeTabId,
  closeTab,
  openDiffTab,
  openFileTab,
  tabs,
} from "../../composables/useEditorTabs";
import { tooltipDirective } from "../../directives/tooltip";
import { __resetSessionFsForTest } from "../../composables/useSessionFs";
import { store } from "../../composables/useCodex";

const mockedInvoke = vi.mocked(invoke);
const root = "D:\\repo";
const aTxt = root + "\\a.txt";
const bTxt = root + "\\b.txt";

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
    __resetEditorTabsForTest();
    __resetSessionFsForTest();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("仅有会话标签时隐藏标签栏，对话区直接可见", () => {
    const wrapper = mountPane();
    expect(wrapper.find(".editor-tabs").exists()).toBe(false);
    expect(wrapper.find(".chat-stub").exists()).toBe(true);
    wrapper.unmount();
  });

  it("打开文件后显示标签栏：会话为 Logo 主标签且不可关闭", async () => {
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
    expect(tabEls[0].classes()).toContain("pinned");
    expect(tabEls[0].find(".editor-tab-logo").exists()).toBe(true);
    expect(tabEls[0].find(".editor-tab-label").exists()).toBe(false);
    expect(tabEls[0].attributes("aria-label")).toBe("会话");
    expect(tabEls[0].find(".editor-tab-close").exists()).toBe(false);
    wrapper.unmount();
  });

  it("关闭最后一个文件标签后标签栏再次隐藏", async () => {
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

    await wrapper.find(".editor-tab-close").trigger("click");
    await settle();
    expect(wrapper.find(".editor-tabs").exists()).toBe(false);
    expect(wrapper.find(".chat-stub").exists()).toBe(true);
    wrapper.unmount();
  });

  it("会话标签右键菜单：关闭其它所有标签，未保存的跳过并提示", async () => {
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
    expect(items).toHaveLength(1);
    expect(items[0].text().trim()).toBe("关闭其它所有标签");

    await items[0].trigger("click");
    await flushPromises();
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
    expect(tabs.map((t) => t.title)).toEqual(["对话", "b.txt"]);
    expect(store.toast).toContain("已跳过 1 个未保存的标签");
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
    expect(labels).toHaveLength(1);
    expect(labels[0].text()).toContain("a.txt");
    expect(wrapper.find(".editor-tab-icon").exists()).toBe(true);
    expect(wrapper.find(".editor-tab-icon svg").exists()).toBe(true);
    expect(wrapper.findAll(".editor-tab")[1].attributes("data-tip")).toBe(
      "a.txt",
    );
    await waitForEl(wrapper, ".text-editor-path");
    expect(wrapper.find(".text-editor-path").text()).toBe("a.txt");
    expect((await viewOf(wrapper)).state.doc.toString()).toBe("hello");
    expect(wrapper.find(".text-editor-dirty").exists()).toBe(false);
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
      root,
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

    activateTab("chat");
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
      root,
      path: "a.txt",
      content: "xhello",
    });
    expect(tabs.some((t) => t.id === aTab.id)).toBe(false);
    expect(activeTabId.value).toBe("chat");
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
      workspace_root: root,
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
});
