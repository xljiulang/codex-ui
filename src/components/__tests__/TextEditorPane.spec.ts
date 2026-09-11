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
import { store } from "../../composables/useCodex";
import {
  __resetEditorTabsForTest,
  openFileTab,
  tabs,
  type FileEditorTab,
} from "../../composables/useEditorTabs";
import { tooltipDirective } from "../../directives/tooltip";

const mockedInvoke = vi.mocked(invoke);
const root = "D:\\repo";

/**
 * 宽松等待上限：`代码格式化` 会 lazy import prettier（standalone + 8 个插件），
 * 并行跑全量用例时首次加载可能超过默认的 5s，偶发导致 vi.waitFor 超时。
 * 用例超时同步放宽（只在真要超时时才会等到这个上限）。
 */
const WAIT_TIMEOUT = 15_000;
vi.setConfig({ testTimeout: 30_000, hookTimeout: 30_000 });

function fileContent(content: string) {
  return { content, validUtf8: true, byteSize: content.length };
}

async function openTab(path: string, content: string): Promise<FileEditorTab> {
  mockedInvoke.mockImplementation((cmd) => {
    if (cmd === "session_fs_read") {
      return Promise.resolve(fileContent(content));
    }
    if (cmd === "clipboard_read_text") {
      return Promise.resolve("pasted-text");
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
    { timeout: WAIT_TIMEOUT, interval: 20 },
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
    { timeout: WAIT_TIMEOUT, interval: 20 },
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
      },
      configurable: true,
    });
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("Markdown 文件默认渲染模式并显示「预览/编辑」切换按钮，普通文件不显示", async () => {
    const mdTab = await openTab("a.md", "# 标题");
    const mdWrapper = await mountEditor(mdTab);
    // 默认渲染模式：按钮显示「编辑」（点击切回编辑）
    expect(
      mdWrapper.find(".text-editor-actions button[aria-label='编辑']").exists(),
    ).toBe(true);
    await mdWrapper
      .find(".text-editor-actions button[aria-label='编辑']")
      .trigger("click");
    await nextTick();
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

  it("滚动事件写回标签 scrollTop，挂载时按标签记录恢复", async () => {
    const tab = await openTab("a.txt", "line1\nline2\nline3");
    tab.scrollTop = 42;
    const wrapper = await mountEditor(tab);
    const view = await viewOf(wrapper);
    expect(view.scrollDOM.scrollTop).toBe(42);

    view.scrollDOM.scrollTop = 77;
    view.scrollDOM.dispatchEvent(new Event("scroll"));
    expect(tab.scrollTop).toBe(77);
    wrapper.unmount();
  });

  it("预览/编辑切换：默认预览渲染当前文档，切编辑保留内容、切回预览正常", async () => {
    const content = "# 标题\n\n**加粗** 正文";
    const tab = await openTab("a.md", content);
    const wrapper = await mountEditor(tab);

    // 默认渲染模式：预览渲染当前文档
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

    await wrapper
      .find(".text-editor-actions button[aria-label='预览']")
      .trigger("click");
    await nextTick();
    expect(wrapper.find(".text-editor-host").attributes("style")).toContain(
      "display: none",
    );
    await waitForEl(wrapper, ".text-editor-preview .md");
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

  it("预览态写入标签状态：卸载重挂载后仍保持预览", async () => {
    const tab = await openTab("a.md", "# 标题");
    const w1 = await mountEditor(tab);
    // 默认渲染模式：初始即为预览态
    expect(tab.markdownPreview).toBe(true);
    expect(
      w1.find(".text-editor-actions button[aria-label='编辑']").exists(),
    ).toBe(true);
    await waitForEl(w1, ".text-editor-preview .md");
    w1.unmount();

    const w2 = await mountEditor(tab);
    expect(tab.markdownPreview).toBe(true);
    expect(
      w2.find(".text-editor-actions button[aria-label='编辑']").exists(),
    ).toBe(true);
    await waitForEl(w2, ".text-editor-preview .md");
    expect(w2.find(".text-editor-preview .md").text()).toContain("标题");
    w2.unmount();
  });

  it("切换标签后预览显示新标签内容（回归：不残留上一标签内容）", async () => {
    mockedInvoke.mockImplementation((cmd, args) => {
      if (cmd === "session_fs_read") {
        const path = (args as { path?: string }).path;
        return Promise.resolve(
          path === "b.md"
            ? fileContent("# B标题\n\nB 正文")
            : fileContent("# A标题\n\nA 正文"),
        );
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    await openFileTab(root, "a.md");
    const aTab = tabs.find(
      (t): t is FileEditorTab => t.kind === "file" && t.title === "a.md",
    )!;
    const wrapper = await mountEditor(aTab);

    // A 默认渲染模式：确认渲染的是 A 内容
    await waitForEl(wrapper, ".text-editor-preview .md");
    expect(wrapper.find(".text-editor-preview .md").text()).toContain("A标题");

    // 切到 B 标签（视图换档完成）：B 默认渲染模式，必须显示 B 内容
    await openFileTab(root, "b.md");
    const bTab = tabs.find(
      (t): t is FileEditorTab => t.kind === "file" && t.title === "b.md",
    )!;
    await wrapper.setProps({ tab: bTab });
    await flushPromises();
    await nextTick();
    await viewOf(wrapper);
    await waitForEl(wrapper, ".text-editor-preview .md");
    await vi.waitFor(
      () => {
        const txt = wrapper.find(".text-editor-preview .md").text();
        expect(txt).toContain("B标题");
        expect(txt).not.toContain("A标题");
      },
      { timeout: WAIT_TIMEOUT, interval: 20 },
    );
    wrapper.unmount();
  });

  it("可格式化文件（json/ts/py/xml/html）右键菜单末尾为「代码格式化」", async () => {
    for (const [name, content] of [
      ["a.json", '{"a":1}'],
      ["a.ts", "const x: number = 1;"],
      ["a.py", "x = 1"],
      ["a.xml", "<root><a>1</a></root>"],
      ["a.html", "<div><p>x</p></div>"],
    ] as const) {
      const tab = await openTab(name, content);
      const wrapper = await mountEditor(tab);
      await openCtx(wrapper);
      const labels = menuLabels(wrapper);
      expect(labels[labels.length - 1], name).toBe("代码格式化");
      wrapper.unmount();
    }
  });

  it("不可格式化文件（txt/无扩展名）不显示「代码格式化」", async () => {
    const tab = await openTab("a.txt", "hello");
    const wrapper = await mountEditor(tab);
    await openCtx(wrapper);
    expect(menuLabels(wrapper)).not.toContain("代码格式化");
    wrapper.unmount();

    const noExtTab = await openTab("noext", "hello");
    const noExtWrapper = await mountEditor(noExtTab);
    await openCtx(noExtWrapper);
    expect(menuLabels(noExtWrapper)).not.toContain("代码格式化");
    noExtWrapper.unmount();
  });

  it("点击「代码格式化」：JSON 规范化（空格/缩进）、置脏、可撤销回退", async () => {
    const tab = await openTab("a.json", '{"a":1,"b":[1,2]}');
    const wrapper = await mountEditor(tab);
    const view = await viewOf(wrapper);

    await openCtx(wrapper);
    await clickMenuItem(wrapper, "代码格式化");

    // 短 JSON 对象在打印宽度内保持单行，但规范化为空格与 4 空格缩进风格
    await vi.waitFor(
      () => {
        expect(view.state.doc.toString()).toBe(
          '{ "a": 1, "b": [1, 2] }',
        );
      },
      { timeout: WAIT_TIMEOUT, interval: 20 },
    );
    expect(tab.dirty).toBe(true);

    await openCtx(wrapper);
    await clickMenuItem(wrapper, "撤销");
    expect(view.state.doc.toString()).toBe('{"a":1,"b":[1,2]}');
    wrapper.unmount();
  });

  it("点击「代码格式化」：XML 展开为多行", async () => {
    const tab = await openTab("a.xml", "<root><a>1</a></root>");
    const wrapper = await mountEditor(tab);
    const view = await viewOf(wrapper);

    await openCtx(wrapper);
    await clickMenuItem(wrapper, "代码格式化");

    expect(view.state.doc.toString()).toBe(
      "<root>\n    <a>1</a>\n</root>",
    );
    wrapper.unmount();
  });

  it("Markdown 文件右键菜单末尾显示「导出 PDF」，普通文件不显示", async () => {
    const mdTab = await openTab("a.md", "# 标题");
    const mdWrapper = await mountEditor(mdTab);
    await openCtx(mdWrapper);
    const mdLabels = menuLabels(mdWrapper);
    expect(mdLabels[mdLabels.length - 1]).toBe("导出 PDF");
    mdWrapper.unmount();

    const txtTab = await openTab("a.txt", "hello");
    const txtWrapper = await mountEditor(txtTab);
    await openCtx(txtWrapper);
    expect(menuLabels(txtWrapper)).not.toContain("导出 PDF");
    txtWrapper.unmount();
  });

  it("点击「导出 PDF」：以当前（含未保存）内容调用导出命令，成功后提示路径", async () => {
    const content = "# 标题\n\n正文";
    const tab = await openTab("a.md", content);
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent(content));
      }
      if (cmd === "export_markdown_pdf") {
        return Promise.resolve("D:\\out\\a.pdf");
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    store.toast = "";
    const wrapper = await mountEditor(tab);

    // 修改文档但不保存：导出必须使用当前缓冲区内容
    const view = await viewOf(wrapper);
    view.dispatch({ changes: { from: 0, insert: "> " } });
    expect(tab.dirty).toBe(true);

    await openCtx(wrapper);
    await clickMenuItem(wrapper, "导出 PDF");

    await vi.waitFor(
      () => {
        expect(mockedInvoke).toHaveBeenCalledWith(
          "export_markdown_pdf",
          expect.objectContaining({
            suggestedName: "a.pdf",
            initialDir: "D:\\repo",
          }),
        );
      },
      { timeout: WAIT_TIMEOUT, interval: 20 },
    );
    const args = mockedInvoke.mock.calls.find(
      (c) => c[0] === "export_markdown_pdf",
    )?.[1] as { html?: string };
    expect(args.html ?? "").toContain("<h1>标题</h1>");
    await vi.waitFor(
      () => {
        expect(store.toast).toBe("已导出 PDF：D:\\out\\a.pdf");
      },
      { timeout: WAIT_TIMEOUT, interval: 20 },
    );
    wrapper.unmount();
  });

  it("非法 JSON 点击「代码格式化」：文档不变并弹出错误提示", async () => {
    store.toast = "";
    const tab = await openTab("a.json", '{"a": }');
    const wrapper = await mountEditor(tab);
    const view = await viewOf(wrapper);

    await openCtx(wrapper);
    await clickMenuItem(wrapper, "代码格式化");

    expect(view.state.doc.toString()).toBe('{"a": }');
    await vi.waitFor(
      () => {
        expect(store.toast).toBe("JSON 语法错误，无法格式化");
      },
      { timeout: WAIT_TIMEOUT, interval: 20 },
    );
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
    expect(mockedInvoke).toHaveBeenCalledWith("clipboard_read_text");
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

    // 默认渲染模式：无需切换，预览态下直接 Ctrl+S 保存
    window.dispatchEvent(
      new KeyboardEvent("keydown", { key: "s", ctrlKey: true }),
    );
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_write", {
      workspace: root,
      path: "a.md",
      content: "# # hi",
    });
    expect(tab.dirty).toBe(false);
    wrapper.unmount();
  });
});
