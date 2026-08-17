import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick } from "vue";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { invoke } from "@tauri-apps/api/core";
import DocxEditorPane from "../DocxEditorPane.vue";
import {
  __resetEditorTabsForTest,
  tabs,
  type DocxEditorTab,
} from "../../composables/useEditorTabs";
import { insertTab } from "../../composables/useTabs";
import { tooltipDirective } from "../../directives/tooltip";

const mockedInvoke = vi.mocked(invoke);
const root = "D:\\repo";

function docxTab(html: string, overrides: Partial<DocxEditorTab> = {}): DocxEditorTab {
  const tab = {
    kind: "docx",
    id: "file:[\"D:\\\\repo\",\"a.docx\"]",
    workspace: root,
    path: "a.docx",
    title: "a.docx",
    icon: "file",
    loading: false,
    error: "",
    dirty: false,
    saving: false,
    status: "",
    byteSize: 1024,
    stale: false,
    initialHtml: html,
    editor: null,
    ...overrides,
  } as unknown as DocxEditorTab;
  insertTab(tab);
  return tab;
}

async function mountEditor(tab: DocxEditorTab) {
  const wrapper = mount(DocxEditorPane, {
    props: { tab },
    global: { directives: { tooltip: tooltipDirective } },
  });
  await flushPromises();
  await nextTick();
  await vi.waitFor(
    () => {
      if (!tab.editor) throw new Error("waiting for editor");
    },
    { timeout: 5000, interval: 20 },
  );
  return wrapper;
}

describe("DocxEditorPane .docx 富文本编辑", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    __resetEditorTabsForTest();
  });

  afterEach(() => {
    for (const t of [...tabs]) {
      if (t.kind === "docx" && t.editor) t.editor.destroy();
    }
  });

  it("加载 HTML 渲染编辑器与工具栏", async () => {
    const tab = docxTab("<h1>标题</h1><p>正文内容</p>");
    const wrapper = await mountEditor(tab);

    expect(wrapper.find(".docx-editor-toolbar").exists()).toBe(true);
    expect(
      wrapper.find(".docx-editor-window.docx-editor-embedded").exists(),
    ).toBe(true);
    expect(wrapper.find(".docx-editor-host .tiptap").exists()).toBe(true);
    expect(wrapper.text()).toContain("标题");
    expect(wrapper.text()).toContain("正文内容");
    expect(wrapper.text()).toContain("Word 文档");
    expect(wrapper.text()).toContain("页眉页脚、批注、修订等高级格式可能丢失");
  });

  it("编辑后置脏；点击保存调用 session_fs_write_bytes 并复位", async () => {
    const tab = docxTab("<p>旧内容</p>");
    const wrapper = await mountEditor(tab);
    tab.editor!.commands.setContent("<p>新内容</p>");
    await nextTick();
    expect(tab.dirty).toBe(true);
    expect(wrapper.find(".docx-editor-dirty").exists()).toBe(true);

    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_write_bytes") {
        return Promise.resolve({ name: "a.docx", path: "a.docx" });
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await wrapper.find(".docx-editor-tool-btn.primary").trigger("click");
    await flushPromises();

    await vi.waitFor(
      () => {
        expect(mockedInvoke).toHaveBeenCalledWith("session_fs_write_bytes", {
          workspace: root,
          path: "a.docx",
          content: expect.stringMatching(/^UEs/),
        });
      },
      { timeout: 5000, interval: 20 },
    );
    expect(tab.dirty).toBe(false);
    expect(tab.status).toContain("已保存");
  });

  it("Ctrl+S 触发保存", async () => {
    const tab = docxTab("<p>内容</p>");
    await mountEditor(tab);
    tab.editor!.commands.setContent("<p>改动</p>");
    await nextTick();
    mockedInvoke.mockResolvedValue({ name: "a.docx", path: "a.docx" });

    window.dispatchEvent(
      new KeyboardEvent("keydown", { key: "s", ctrlKey: true, bubbles: true }),
    );
    await vi.waitFor(
      () => {
        expect(mockedInvoke).toHaveBeenCalledWith(
          "session_fs_write_bytes",
          expect.anything(),
        );
      },
      { timeout: 5000, interval: 20 },
    );
    expect(tab.dirty).toBe(false);
  });

  it("切换标签只保留当前标签的编辑器 DOM 与内容", async () => {
    const tabA = docxTab("<h1>文档A</h1><p>AAA</p>");
    const tabB = docxTab("<h1>文档B</h1><p>BBB</p>", {
      id: 'file:["D:\\repo","b.docx"]',
      path: "b.docx",
      title: "b.docx",
    });
    const wrapper = await mountEditor(tabA);
    expect(wrapper.findAll(".docx-editor-host .tiptap")).toHaveLength(1);
    expect(wrapper.text()).toContain("文档A");

    await wrapper.setProps({ tab: tabB });
    await flushPromises();
    await nextTick();
    await vi.waitFor(
      () => {
        expect(wrapper.findAll(".docx-editor-host .tiptap")).toHaveLength(1);
      },
      { timeout: 5000, interval: 20 },
    );
    expect(wrapper.text()).toContain("文档B");
    expect(wrapper.text()).not.toContain("文档A");

    await wrapper.setProps({ tab: tabA });
    await flushPromises();
    await nextTick();
    await vi.waitFor(
      () => {
        expect(wrapper.findAll(".docx-editor-host .tiptap")).toHaveLength(1);
      },
      { timeout: 5000, interval: 20 },
    );
    expect(wrapper.text()).toContain("文档A");
    expect(wrapper.text()).not.toContain("文档B");
    expect(tabA.editor).not.toBeNull();
    expect(tabB.editor).not.toBeNull();
    expect(tabA.editor).not.toBe(tabB.editor);
  });

  it("加载失败显示错误态，不进入编辑器", async () => {
    const tab = docxTab("<p>x</p>", {
      error: "无法解析该 .docx 文件",
      initialHtml: null,
    });
    const wrapper = mount(DocxEditorPane, {
      props: { tab },
      global: { directives: { tooltip: tooltipDirective } },
    });
    await flushPromises();
    expect(wrapper.text()).toContain("无法编辑该文件");
    expect(wrapper.find(".docx-editor-toolbar").exists()).toBe(false);
    expect(tab.editor).toBeNull();
  });
});
