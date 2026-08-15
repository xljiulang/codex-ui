import { describe, expect, it, vi, beforeEach } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, openNewSession: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import AppHeader from "../AppHeader.vue";
import { tooltipDirective } from "../../directives/tooltip";
import { openNewSession, store } from "../../composables/useCodex";

const mockedOpenNewSession = vi.mocked(openNewSession);
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
    mockedOpenNewSession.mockClear();
    mockedInvoke.mockResolvedValue("D:/project");
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
    expect(mockedOpenNewSession).not.toHaveBeenCalled();
  });

  it("设置打开时点新建会话：调用统一新建入口", async () => {
    store.showSettings = true;
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="新建会话"]').trigger("click");
    expect(mockedOpenNewSession).toHaveBeenCalledTimes(1);
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
    mockedOpenNewSession.mockClear();
  });

  it("点击新建会话先弹文件夹选择器，选中目录后写入并新建", async () => {
    mockedInvoke.mockResolvedValue("D:/project");
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="新建会话"]').trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("pick_directory", {
      initialDir: "D:/repo",
    });
    expect(mockedOpenNewSession).toHaveBeenCalledWith("D:/project");
    expect(mockedOpenNewSession).toHaveBeenCalledTimes(1);
  });

  it("有会话时目录选择器初始目录为当前会话工作目录", async () => {
    store.currentThreadId = "t1";
    store.currentThreadCwd = "D:/session";
    mockedInvoke.mockResolvedValue("D:/project");
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="新建会话"]').trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("pick_directory", {
      initialDir: "D:/session",
    });
  });

  it("取消选择文件夹：流程直接结束（不新建、不聚焦、不切 Tab）", async () => {
    mockedInvoke.mockResolvedValue(null);
    store.showSettings = true;
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="新建会话"]').trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("pick_directory", {
      initialDir: "D:/repo",
    });
    expect(mockedOpenNewSession).not.toHaveBeenCalled();
    expect(store.panelTab).toBe("history");
    expect(store.showSettings).toBe(true);
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
    expect(mockedOpenNewSession).not.toHaveBeenCalled();
    resolveDir(null);
    await flushPromises();
    expect(btn.attributes("disabled")).toBeUndefined();
    expect(mockedOpenNewSession).not.toHaveBeenCalled();
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
