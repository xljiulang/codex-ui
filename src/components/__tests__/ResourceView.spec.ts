import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return {
    ...mod,
    refreshThreads: vi.fn(),
  };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { invoke } from "@tauri-apps/api/core";
import ResourceView from "../ResourceView.vue";
import { store } from "../../composables/useCodex";
import { __resetSessionFsForTest } from "../../composables/useSessionFs";
import { tooltipDirective } from "../../directives/tooltip";
import type { FsEntry } from "../../lib/sessionFs";

const mockedInvoke = vi.mocked(invoke);
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

function mockFs() {
  mockedInvoke.mockImplementation((cmd, args) => {
    if (cmd === "session_fs_metadata") {
      const path = (args as { path?: string }).path;
      if (path === aTxt.path) return Promise.resolve(aTxt);
      return Promise.resolve(rootEntry);
    }
    if (cmd === "session_fs_list") {
      const dir = (args as { dir?: string }).dir;
      if (dir === rootPath) return Promise.resolve([srcDir, nodeModules, aTxt]);
      if (dir === srcDir.path) return Promise.resolve([mainTs, picPng]);
      return Promise.resolve([]);
    }
    if (cmd === "session_fs_search") {
      const query = (args as { query?: string }).query ?? "";
      if (query.toLowerCase().includes("main")) return Promise.resolve([mainTs]);
      if (query.toLowerCase().includes("src")) return Promise.resolve([srcDir]);
      return Promise.resolve([]);
    }
    if (cmd === "session_fs_icons") return Promise.resolve([]);
    if (cmd === "session_fs_probe_text") {
      const path = (args as { path?: string }).path;
      return Promise.resolve(path !== picPng.path);
    }
    if (cmd === "workspace_dir") return Promise.resolve(rootPath);
    if (cmd === "session_fs_rename") return Promise.resolve({ ...aTxt, name: "b.txt" });
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
  const wrapper = mount(ResourceView, { props: { active } });
  await flushPromises();
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
}

async function clickCtxItem(wrapper: VueWrapper, label: string) {
  const btn = wrapper
    .findAll(".ctx-menu-item")
    .find((b) => b.text().trim() === label);
  expect(btn).toBeTruthy();
  await btn!.trigger("click");
  await flushPromises();
}

describe("ResourceView 文件树", () => {
  beforeEach(() => {
    store.server.workspace = rootPath;
    store.currentThreadCwd = null;
    store.newChatCwd = null;
    store.attachments = [];
    store.toast = "";
    mockedInvoke.mockClear();
    mockFs();
    __resetSessionFsForTest();
  });

  afterEach(() => {
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
      root: rootPath,
      path: rootPath,
    });
    wrapper.unmount();
  });

  it("currentThreadCwd 为空字符串时回退启动工作目录", async () => {
    store.currentThreadCwd = "";
    const wrapper = await mountPanel();
    expect(wrapper.find(".resource-root").text()).toContain("codex-ui");
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_metadata", {
      root: rootPath,
      path: rootPath,
    });
    wrapper.unmount();
  });

  it("工作目录均未就绪时用 workspace_dir 兜底", async () => {
    store.currentThreadCwd = "";
    store.newChatCwd = "";
    store.server.workspace = "";
    const wrapper = await mountPanel();
    expect(wrapper.find(".resource-root").text()).toContain("codex-ui");
    expect(mockedInvoke).toHaveBeenCalledWith("workspace_dir");
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_metadata", {
      root: rootPath,
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

    const img = wrapper.find(".resource-row.resource-file .resource-icon-img");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toBe("data:image/png;base64,ICON");
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
    wrapper.unmount();
  });

  it("文本文件「打开」：调用 open_text_editor 打开编辑窗口", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-file");
    await clickCtxItem(wrapper, "打开");
    expect(mockedInvoke).toHaveBeenCalledWith("open_text_editor", {
      root: rootPath,
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

  it("单击文本文件行调用 open_text_editor 打开编辑器", async () => {
    const wrapper = await mountPanel();
    await wrapper.find(".resource-row.resource-file").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("open_text_editor", {
      root: rootPath,
      path: aTxt.path,
    });
    wrapper.unmount();
  });

  it("单击非文本文件行：提示无法打开且不触发预览", async () => {
    const wrapper = await mountPanel();
    await wrapper.findAll(".resource-row.resource-dir")[0].trigger("click");
    await flushPromises();
    const picRow = wrapper
      .findAll(".resource-row.resource-file")
      .find((w) => w.text().includes("pic.png"));
    expect(picRow).toBeTruthy();
    await picRow!.trigger("click");
    await flushPromises();
    expect(store.toast).toContain("该文件不是文本文件，无法打开");
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "open_text_editor",
      expect.anything(),
    );
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
      "open_text_editor",
      expect.anything(),
    );
    wrapper.unmount();
  });

  it("目录右键菜单项与顺序", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-dir");
    const labels = wrapper.findAll(".ctx-menu-item").map((b) => b.text().trim());
    expect(labels).toEqual([
      "复制",
      "粘贴",
      "删除",
      "重命名",
      "添加为会话附件",
      "在资源管理器中打开",
    ]);
    wrapper.unmount();
  });

  it("根节点右键菜单只含粘贴与在资源管理器中打开", async () => {
    const wrapper = await mountPanel();
    await openRowCtx(wrapper, ".resource-row.resource-root");
    const labels = wrapper.findAll(".ctx-menu-item").map((b) => b.text().trim());
    expect(labels).toEqual(["粘贴", "在资源管理器中打开"]);
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
      root: rootPath,
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
      root: rootPath,
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
      root: rootPath,
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
      root: rootPath,
      destDir: srcDir.path,
      sources: [aTxt.path],
    });
    wrapper.unmount();
  });

  it("添加为会话附件：调用全局入口并携带协议附件", async () => {
    const addAttachment = vi.fn();
    (window as unknown as { __CODEX_UI_ADD_ATTACHMENT__?: unknown }).__CODEX_UI_ADD_ATTACHMENT__ =
      addAttachment;
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

  it("搜索：防抖后调用 session_fs_search，结果显示，清除恢复树", async () => {
    vi.useFakeTimers();
    const wrapper = await mountPanel();
    await wrapper.find(".history-search").setValue("main");
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();

    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_search", {
      root: rootPath,
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
    expect(mockedInvoke).toHaveBeenCalledWith("open_text_editor", {
      root: rootPath,
      path: mainTs.path,
    });
    wrapper.unmount();
  });
});
