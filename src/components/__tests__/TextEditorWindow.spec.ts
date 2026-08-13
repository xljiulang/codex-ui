import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

const h = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  onCloseRequested: vi.fn(),
  destroy: vi.fn(),
  setTitle: vi.fn(),
  closeHandler: undefined as
    | undefined
    | ((event: { preventDefault: () => void }) => void | Promise<void>),
  openHandler: undefined as
    | undefined
    | ((ev: { payload: { root: string; path: string } }) => void),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: h.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, cb: (ev: { payload: unknown }) => void) => {
    if (event === "text-editor/open") {
      h.openHandler = cb as never;
    }
    return Promise.resolve(() => {});
  }),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    label: "text-editor",
    onCloseRequested: h.onCloseRequested,
    destroy: h.destroy,
    setTitle: h.setTitle,
  }),
}));

import TextEditorWindow from "../TextEditorWindow.vue";
import type { EditorView } from "@codemirror/view";

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

function getView(wrapper: Awaited<ReturnType<typeof mount>>): EditorView {
  const host = wrapper.find(".text-editor-host").element as HTMLDivElement & {
    __cmView?: EditorView;
  };
  if (!host.__cmView) throw new Error("editor view not mounted");
  return host.__cmView;
}

describe("TextEditorWindow 文本编辑器", () => {
  beforeEach(() => {
    h.invoke.mockReset();
    h.listen.mockClear();
    h.onCloseRequested.mockReset();
    h.onCloseRequested.mockImplementation(async (cb) => {
      h.closeHandler = cb;
      return () => {};
    });
    h.destroy.mockReset();
    h.destroy.mockResolvedValue(undefined);
    h.setTitle.mockReset();
    h.setTitle.mockResolvedValue(undefined);
    h.closeHandler = undefined;
    h.openHandler = undefined;
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("加载文本文件并渲染编辑器", async () => {
    h.invoke.mockImplementation((cmd: string) => {
      if (cmd === "take_text_editor_params") {
        return Promise.resolve({ root, path: aTxt });
      }
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mount(TextEditorWindow);
    await flushPromises();
    expect(wrapper.find(".text-editor-path").text()).toBe(aTxt);
    expect(wrapper.find(".cm-content").exists()).toBe(true);
    expect(getView(wrapper).state.doc.toString()).toBe("hello");
    expect(wrapper.find(".text-editor-dirty").exists()).toBe(false);
    expect(h.setTitle).toHaveBeenCalledWith("a.txt");
    wrapper.unmount();
  });

  it("编辑后出现未保存标记，保存调用 session_fs_write 并还原 CRLF/BOM", async () => {
    h.invoke.mockImplementation((cmd: string) => {
      if (cmd === "take_text_editor_params") {
        return Promise.resolve({ root, path: aTxt });
      }
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("\uFEFFa\r\nb\r\n"));
      }
      if (cmd === "session_fs_write") {
        return Promise.resolve({});
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mount(TextEditorWindow);
    await flushPromises();

    const view = getView(wrapper);
    view.dispatch({
      changes: { from: 0, insert: "x" },
      selection: { anchor: 1 },
    });
    await flushPromises();
    expect(wrapper.find(".text-editor-dirty").exists()).toBe(true);

    await wrapper
      .find(".text-editor-actions .text-editor-icon-btn.primary")
      .trigger("click");
    await flushPromises();
    expect(h.invoke).toHaveBeenCalledWith("session_fs_write", {
      root,
      path: aTxt,
      content: "\uFEFFxa\r\nb\r\n",
    });
    expect(wrapper.find(".text-editor-dirty").exists()).toBe(false);
    expect(wrapper.find(".text-editor-status-save").text()).toContain("已保存");
    wrapper.unmount();
  });

  it("非 UTF-8 文件只读：显示提示条且保存按钮禁用", async () => {
    h.invoke.mockImplementation((cmd: string) => {
      if (cmd === "take_text_editor_params") {
        return Promise.resolve({ root, path: aTxt });
      }
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("abc", false));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mount(TextEditorWindow);
    await flushPromises();
    expect(wrapper.find(".text-editor-banner").exists()).toBe(true);
    const saveBtn = wrapper.find(
      ".text-editor-actions .text-editor-icon-btn.primary",
    );
    expect(saveBtn.attributes("disabled")).toBeDefined();
    wrapper.unmount();
  });

  it("有未保存更改时关闭窗口弹出确认，放弃后销毁窗口", async () => {
    h.invoke.mockImplementation((cmd: string) => {
      if (cmd === "take_text_editor_params") {
        return Promise.resolve({ root, path: aTxt });
      }
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mount(TextEditorWindow);
    await flushPromises();
    getView(wrapper).dispatch({
      changes: { from: 0, insert: "x" },
      selection: { anchor: 1 },
    });
    await flushPromises();

    const preventDefault = vi.fn();
    await h.closeHandler!({ preventDefault });
    await flushPromises();
    expect(preventDefault).toHaveBeenCalled();
    expect(wrapper.find(".text-editor-overlay").exists()).toBe(true);

    await wrapper
      .findAll(".text-editor-confirm-actions .text-editor-btn")
      .find((b) => b.text() === "放弃更改")!
      .trigger("click");
    await flushPromises();
    expect(h.destroy).toHaveBeenCalled();
    wrapper.unmount();
  });

  it("有未保存更改时切换文件弹出确认，保存后切换", async () => {
    h.invoke.mockImplementation((cmd: string) => {
      if (cmd === "take_text_editor_params") {
        return Promise.resolve({ root, path: aTxt });
      }
      if (cmd === "session_fs_read") {
        return Promise.resolve(fileContent("hello"));
      }
      if (cmd === "session_fs_write") {
        return Promise.resolve({});
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const wrapper = mount(TextEditorWindow);
    await flushPromises();
    getView(wrapper).dispatch({
      changes: { from: 0, insert: "x" },
      selection: { anchor: 1 },
    });
    await flushPromises();

    h.openHandler!({ payload: { root, path: bTxt } });
    await flushPromises();
    expect(wrapper.find(".text-editor-overlay").exists()).toBe(true);

    await wrapper
      .findAll(".text-editor-confirm-actions .text-editor-btn")
      .find((b) => b.text() === "保存并切换")!
      .trigger("click");
    await flushPromises();
    expect(h.invoke).toHaveBeenCalledWith("session_fs_write", {
      root,
      path: aTxt,
      content: "xhello",
    });
    expect(h.invoke).toHaveBeenCalledWith("session_fs_read", {
      root,
      path: bTxt,
    });
    expect(wrapper.find(".text-editor-path").text()).toBe(bTxt);
    expect(h.setTitle).toHaveBeenCalledWith("b.txt");
    wrapper.unmount();
  });
});
