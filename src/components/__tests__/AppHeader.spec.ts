import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";

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
  });

  it("设置按钮无 active 态", async () => {
    store.showSettings = true;
    const wrapper = mountHeader();
    expect(wrapper.find('button[aria-label="设置"]').classes()).not.toContain(
      "active",
    );
  });
});

describe("AppHeader 工作目录选择", () => {
  beforeEach(() => {
    store.currentThreadId = null;
    store.currentThreadCwd = null;
    store.server.workspace = "D:/repo";
    store.newChatCwd = null;
    mockedInvoke.mockReset();
  });

  it("新会话态点击工作目录打开目录选择器，选中后更新显示", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) =>
      cmd === "pick_directory" ? "D:/project" : undefined,
    );
    const wrapper = mountHeader();
    const chip = wrapper.find("button.brand-cwd");
    // 芯片只显示文件夹名，完整路径在 tooltip
    expect(chip.text()).toContain("repo");
    expect(chip.attributes("data-tip")).toBe("D:/repo");
    await chip.trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("pick_directory");
    expect(store.newChatCwd).toBe("D:/project");
    expect(wrapper.find("button.brand-cwd").text()).toContain("project");
  });

  it("新会话态 cwd tooltip 显示完整路径且带 pickable 样式", () => {
    const wrapper = mountHeader();
    const chip = wrapper.find("button.brand-cwd");
    expect(chip.attributes("data-tip")).toBe("D:/repo");
    expect(chip.classes()).toContain("pickable");
  });

  it("工作目录与新建会话黏连成组，设置按钮靠最右", () => {
    const wrapper = mountHeader();
    const group = wrapper.find(".cwd-group");
    expect(group.find("button.brand-cwd").exists()).toBe(true);
    expect(group.find('button[aria-label="新建会话"]').exists()).toBe(true);
    expect(group.find('button[aria-label="新建会话"]').classes()).toContain(
      "cwd-attach",
    );
    const actions = wrapper.findAll(".header-actions > *");
    expect(actions[actions.length - 1].attributes("aria-label")).toBe("设置");
  });

  it("根路径原样显示为工作目录文本", () => {
    store.server.workspace = "C:/";
    const wrapper = mountHeader();
    const chip = wrapper.find("button.brand-cwd");
    expect(chip.text()).toContain("C:/");
    expect(chip.attributes("data-tip")).toBe("C:/");
  });

  it("会话进行中点击 cwd 无任何行为（只读展示）", async () => {
    store.currentThreadId = "t1";
    store.currentThreadCwd = "D:/thread";
    store.newChatCwd = "D:/custom"; // 会话态应忽略残留的新对话目录
    const wrapper = mountHeader();
    const chip = wrapper.find("button.brand-cwd");
    expect(chip.text()).toContain("thread");
    expect(chip.attributes("data-tip")).toBe("D:/thread");
    expect(chip.classes()).toContain("readonly");
    expect(chip.classes()).not.toContain("pickable");
    mockedInvoke.mockResolvedValue(undefined);
    await chip.trigger("click");
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "open_url",
      expect.anything(),
    );
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "pick_directory",
      expect.anything(),
    );
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
