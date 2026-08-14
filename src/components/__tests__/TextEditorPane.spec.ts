import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick } from "vue";
import type { EditorView } from "@codemirror/view";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { invoke } from "@tauri-apps/api/core";
import TextEditorPane from "../TextEditorPane.vue";
import {
  __resetEditorTabsForTest,
  openFileTab,
  tabs,
  type FileEditorTab,
} from "../../composables/useEditorTabs";
import { tooltipDirective } from "../../directives/tooltip";

const mockedInvoke = vi.mocked(invoke);
const root = "D:\\repo";

function fileContent(content: string) {
  return { content, validUtf8: true, byteSize: content.length };
}

async function openTab(path: string, content: string): Promise<FileEditorTab> {
  mockedInvoke.mockImplementation((cmd) => {
    if (cmd === "session_fs_read") {
      return Promise.resolve(fileContent(content));
    }
    return Promise.reject(new Error(`unexpected ${cmd}`));
  });
  await openFileTab(root, path);
  const name = path.split(/[\\/]/).pop();
  const tab = tabs.find((t) => t.kind === "file" && t.title === name);
  if (!tab || tab.kind !== "file") throw new Error("file tab not found");
  return tab;
}

async function mountEditor(tab: FileEditorTab) {
  const wrapper = mount(TextEditorPane, {
    props: { tab },
    global: { directives: { tooltip: tooltipDirective } },
  });
  await flushPromises();
  await nextTick();
  return wrapper;
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

async function waitForEl(
  wrapper: Awaited<ReturnType<typeof mount>>,
  selector: string,
) {
  await vi.waitFor(
    () => {
      expect(wrapper.find(selector).exists()).toBe(true);
    },
    { timeout: 5000, interval: 20 },
  );
}

async function openCtx(wrapper: Awaited<ReturnType<typeof mount>>) {
  await wrapper.find(".text-editor-host .cm-content").trigger("contextmenu", {
    clientX: 80,
    clientY: 80,
  });
  await nextTick();
}

/** happy-dom 无真实布局，桩掉 posAtCoords：返回 null 表示点击不改变选区 */
function stubPosAtCoords(view: EditorView, pos: number | null) {
  view.posAtCoords = (() => pos) as typeof view.posAtCoords;
}

function menuLabels(wrapper: Awaited<ReturnType<typeof mount>>): string[] {
  return wrapper.findAll(".ctx-menu-item").map((b) => b.text().trim());
}

async function clickMenuItem(
  wrapper: Awaited<ReturnType<typeof mount>>,
  label: string,
) {
  const btn = wrapper
    .findAll(".ctx-menu-item")
    .find((b) => b.text().trim() === label);
  expect(btn).toBeTruthy();
  await btn!.trigger("click");
  await flushPromises();
}

describe("TextEditorPane 右键菜单与 Markdown 预览", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetEditorTabsForTest();
    Object.defineProperty(navigator, "clipboard", {
      value: {
        writeText: vi.fn().mockResolvedValue(undefined),
        readText: vi.fn().mockResolvedValue("pasted-text"),
      },
      configurable: true,
    });
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("Markdown 文件显示「预览」切换按钮，普通文件不显示", async () => {
    const mdTab = await openTab("a.md", "# 标题");
    const mdWrapper = await mountEditor(mdTab);
    expect(
      mdWrapper.find(".text-editor-actions button[aria-label='预览']").exists(),
    ).toBe(true);
    mdWrapper.unmount();

    const txtTab = await openTab("a.txt", "hello");
    const txtWrapper = await mountEditor(txtTab);
    expect(
      txtWrapper.find(".text-editor-actions button[aria-label='预览']").exists(),
    ).toBe(false);
    txtWrapper.unmount();
  });

  it("预览/编辑切换：预览渲染当前文档，切回编辑保留内容与脏状态", async () => {
    const content = "# 标题\n\n**加粗** 正文";
    const tab = await openTab("a.md", content);
    const wrapper = await mountEditor(tab);
    const view = await viewOf(wrapper);
    expect(view.state.doc.toString()).toBe(content);

    await wrapper
      .find(".text-editor-actions button[aria-label='预览']")
      .trigger("click");
    await nextTick();
    expect(wrapper.find(".text-editor-host").attributes("style")).toContain(
      "display: none",
    );
    await waitForEl(wrapper, ".text-editor-preview .md");
    expect(wrapper.find(".text-editor-preview .md").text()).toContain("标题");
    expect(wrapper.find(".text-editor-preview .md").text()).toContain("正文");

    await wrapper
      .find(".text-editor-actions button[aria-label='编辑']")
      .trigger("click");
    await nextTick();
    const style = wrapper.find(".text-editor-host").attributes("style") ?? "";
    expect(style).not.toContain("display: none");
    expect((await viewOf(wrapper)).state.doc.toString()).toBe(content);
    expect(tab.dirty).toBe(false);
    wrapper.unmount();
  });

  it("右键菜单项：无选区隐藏剪切/复制，有选区显示完整菜单", async () => {
    const tab = await openTab("a.txt", "hello world");
    const wrapper = await mountEditor(tab);
    const view = await viewOf(wrapper);

    stubPosAtCoords(view, 3);
    await openCtx(wrapper);
    expect(menuLabels(wrapper)).toEqual([
      "撤销",
      "重做",
      "粘贴",
      "全选",
      "查找/替换",
    ]);
    // 右键定位光标：点击处不在选区时，锚点移动到点击位置
    expect(view.state.selection.main.anchor).toBe(3);

    // 选中前 5 个字符后右键：剪切/复制出现
    stubPosAtCoords(view, 2);
    view.dispatch({ selection: { anchor: 0, head: 5 } });
    await openCtx(wrapper);
    expect(menuLabels(wrapper)).toEqual([
      "撤销",
      "重做",
      "剪切",
      "复制",
      "粘贴",
      "全选",
      "查找/替换",
    ]);
    wrapper.unmount();
  });

  it("复制：写入剪贴板；剪切：写入并删除选区；粘贴：读剪贴板插入并置脏", async () => {
    const tab = await openTab("a.txt", "hello");
    const wrapper = await mountEditor(tab);
    const view = await viewOf(wrapper);

    // 点击落在选区内：不改变选区，剪切/复制可用
    stubPosAtCoords(view, 2);
    view.dispatch({ selection: { anchor: 0, head: 5 } });
    await openCtx(wrapper);
    await clickMenuItem(wrapper, "复制");
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith("hello");

    view.dispatch({ selection: { anchor: 0, head: 5 } });
    await openCtx(wrapper);
    await clickMenuItem(wrapper, "剪切");
    expect(view.state.doc.toString()).toBe("");

    // 空文档下右键点击不移动光标（posAtCoords 返回 null）
    stubPosAtCoords(view, null);
    view.dispatch({ selection: { anchor: 0 } });
    await openCtx(wrapper);
    await clickMenuItem(wrapper, "粘贴");
    expect(navigator.clipboard.readText).toHaveBeenCalled();
    expect(view.state.doc.toString()).toBe("pasted-text");
    expect(tab.dirty).toBe(true);
    wrapper.unmount();
  });

  it("全选与撤销：菜单命令作用于文档", async () => {
    const tab = await openTab("a.txt", "hello");
    const wrapper = await mountEditor(tab);
    const view = await viewOf(wrapper);

    await openCtx(wrapper);
    await clickMenuItem(wrapper, "全选");
    expect(view.state.selection.main.from).toBe(0);
    expect(view.state.selection.main.to).toBe(5);

    view.dispatch({ changes: { from: 0, insert: "X" } });
    expect(view.state.doc.toString()).toBe("Xhello");
    await openCtx(wrapper);
    await clickMenuItem(wrapper, "撤销");
    expect(view.state.doc.toString()).toBe("hello");
    wrapper.unmount();
  });

  it("外部点击与 Escape 关闭右键菜单", async () => {
    const tab = await openTab("a.txt", "hello");
    const wrapper = await mountEditor(tab);

    await openCtx(wrapper);
    expect(wrapper.find(".ctx-menu").exists()).toBe(true);
    window.dispatchEvent(new Event("click"));
    await nextTick();
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);

    await openCtx(wrapper);
    expect(wrapper.find(".ctx-menu").exists()).toBe(true);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await nextTick();
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
    wrapper.unmount();
  });

  it("预览态下 Ctrl+S 仍可保存", async () => {
    const tab = await openTab("a.md", "# hi");
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("# hi"));
      }
      if (cmd === "session_fs_write") {
        return Promise.resolve({});
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = await mountEditor(tab);
    const view = await viewOf(wrapper);
    view.dispatch({ changes: { from: 0, insert: "# " } });
    expect(tab.dirty).toBe(true);

    await wrapper
      .find(".text-editor-actions button[aria-label='预览']")
      .trigger("click");
    await nextTick();
    window.dispatchEvent(
      new KeyboardEvent("keydown", { key: "s", ctrlKey: true }),
    );
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_write", {
      root,
      path: "a.md",
      content: "# # hi",
    });
    expect(tab.dirty).toBe(false);
    wrapper.unmount();
  });
});
