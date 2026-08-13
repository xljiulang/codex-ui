import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, newEmptyChat: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import AppHeader from "../AppHeader.vue";
import { tooltipDirective } from "../../directives/tooltip";
import { newEmptyChat, store } from "../../composables/useCodex";

const mockedNewChat = vi.mocked(newEmptyChat);
const mockedInvoke = vi.mocked(invoke);

function mountHeader() {
  return mount(AppHeader, {
    global: { directives: { tooltip: tooltipDirective } },
  });
}

describe("AppHeader 导航", () => {
  beforeEach(() => {
    store.showSettings = false;
    store.currentThreadId = null;
    store.currentThreadCwd = null;
    store.server.workspace = "";
    store.panelTab = "history";
    mockedNewChat.mockClear();
  });

  it("点设置打开设置页", async () => {
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(store.showSettings).toBe(true);
  });

  it("设置页再点设置关闭设置，回到原对话且不新建/不中断", async () => {
    store.showSettings = true;
    store.currentThreadId = "t1";
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(store.showSettings).toBe(false);
    expect(store.currentThreadId).toBe("t1");
    expect(mockedNewChat).not.toHaveBeenCalled();
  });

  it("设置打开时点新建会话：关闭设置并新建", async () => {
    store.showSettings = true;
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="新建会话"]').trigger("click");
    expect(store.showSettings).toBe(false);
    expect(mockedNewChat).toHaveBeenCalledTimes(1);
    expect(store.panelTab).toBe("resources");
  });

  it("设置按钮无 active 态", async () => {
    store.showSettings = true;
    const wrapper = mountHeader();
    expect(wrapper.find('button[aria-label="设置"]').classes()).not.toContain(
      "active",
    );
  });
});

describe("AppHeader 新建会话选择工作目录", () => {
  beforeEach(() => {
    store.currentThreadId = null;
    store.currentThreadCwd = null;
    store.server.workspace = "D:/repo";
    store.newChatCwd = null;
    store.panelTab = "history";
    mockedInvoke.mockReset();
    mockedNewChat.mockClear();
  });

  it("点击新建会话先弹文件夹选择器，选中目录后写入并新建", async () => {
    mockedInvoke.mockResolvedValue("D:/project");
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="新建会话"]').trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("pick_directory");
    expect(store.newChatCwd).toBe("D:/project");
    expect(mockedNewChat).toHaveBeenCalledTimes(1);
    expect(store.panelTab).toBe("resources");
  });

  it("取消选择文件夹仍新建会话，沿用当前工作目录", async () => {
    mockedInvoke.mockResolvedValue(null);
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="新建会话"]').trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("pick_directory");
    expect(store.newChatCwd).toBeNull();
    expect(mockedNewChat).toHaveBeenCalledTimes(1);
    expect(store.panelTab).toBe("resources");
  });

  it("选择器打开期间按钮禁用，重复点击不会再次弹窗", async () => {
    let resolveDir!: (v: string | null) => void;
    mockedInvoke.mockImplementation(
      () =>
        new Promise<string | null>((resolve) => {
          resolveDir = resolve;
        }),
    );
    const wrapper = mountHeader();
    const btn = wrapper.find('button[aria-label="新建会话"]');
    await btn.trigger("click");
    await wrapper.vm.$nextTick();
    expect(btn.attributes("disabled")).toBeDefined();
    await btn.trigger("click");
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
    expect(mockedNewChat).not.toHaveBeenCalled();
    resolveDir(null);
    await flushPromises();
    expect(btn.attributes("disabled")).toBeUndefined();
    expect(mockedNewChat).toHaveBeenCalledTimes(1);
  });

  it("工作目录按钮已移除，头部只剩新建会话与设置两个图标按钮", () => {
    const wrapper = mountHeader();
    expect(wrapper.find("button.brand-cwd").exists()).toBe(false);
    expect(wrapper.find(".cwd-group").exists()).toBe(false);
    const actions = wrapper.findAll(".header-actions > *");
    expect(actions.map((a) => a.attributes("aria-label"))).toEqual([
      "新建会话",
      "设置",
    ]);
  });
});

describe("AppHeader 新建会话聚焦输入框", () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__;
    document.body.innerHTML = "";
  });

  it("优先通过 Tiptap 编辑器实例聚焦", async () => {
    const focus = vi.fn();
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ = {
      commands: { focus },
    };
    const wrapper = mountHeader();

    await wrapper.find('button[aria-label="新建会话"]').trigger("click");
    await wrapper.vm.$nextTick();

    expect(focus).toHaveBeenCalledTimes(1);
  });

  it("无编辑器实例时兜底聚焦 .ProseMirror 元素", async () => {
    const composer = document.createElement("div");
    composer.className = "composer";
    const editor = document.createElement("div");
    editor.className = "ProseMirror";
    composer.appendChild(editor);
    document.body.appendChild(composer);
    const wrapper = mountHeader();

    await wrapper.find('button[aria-label="新建会话"]').trigger("click");
    await wrapper.vm.$nextTick();

    expect(document.activeElement).toBe(editor);
  });
});
