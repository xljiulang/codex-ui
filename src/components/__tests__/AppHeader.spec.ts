import { describe, expect, it, vi, beforeEach } from "vitest";
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
    store.showHistory = false;
    store.currentThreadId = null;
    store.currentThreadCwd = null;
    store.server.workspace = "";
    mockedNewChat.mockClear();
  });

  it("聊天页点设置打开设置页，且关闭历史面板", async () => {
    store.showHistory = true;
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(store.showSettings).toBe(true);
    expect(store.showHistory).toBe(false);
  });

  it("历史记录按钮手动 toggle，设置面板互斥隐藏", async () => {
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="历史记录"]').trigger("click");
    expect(store.showHistory).toBe(true);
    await wrapper.find('button[aria-label="历史记录"]').trigger("click");
    expect(store.showHistory).toBe(false);
    store.showHistory = true;
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(store.showHistory).toBe(false);
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

  it("历史面板打开时点设置进设置页，再点一次回到对话", async () => {
    store.showHistory = true;
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(store.showSettings).toBe(true);
    expect(store.showHistory).toBe(false);
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(store.showSettings).toBe(false);
  });

  it("设置打开时点新建对话：关闭设置并新建", async () => {
    store.showSettings = true;
    const wrapper = mountHeader();
    await wrapper.find('button[aria-label="新建对话"]').trigger("click");
    expect(store.showSettings).toBe(false);
    expect(mockedNewChat).toHaveBeenCalledTimes(1);
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

  it("新对话态点击工作目录打开目录选择器，选中后更新显示", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) =>
      cmd === "pick_directory" ? "D:/project" : undefined,
    );
    const wrapper = mountHeader();
    const chip = wrapper.find("button.brand-cwd");
    expect(chip.text()).toContain("D:/repo");
    await chip.trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("pick_directory");
    expect(store.newChatCwd).toBe("D:/project");
    expect(wrapper.find("button.brand-cwd").text()).toContain("D:/project");
  });

  it("新对话态 cwd tooltip 提示点击可修改且带 pickable 样式", () => {
    const wrapper = mountHeader();
    const chip = wrapper.find("button.brand-cwd");
    expect(chip.attributes("data-tip")).toBe("点击可修改工作目录");
    expect(chip.classes()).toContain("pickable");
  });

  it("自定义目录后显示 × 恢复按钮，点击恢复默认", async () => {
    store.newChatCwd = "D:/custom";
    const wrapper = mountHeader();
    const reset = wrapper.find(
      'button.brand-cwd-reset[aria-label="恢复默认工作目录"]',
    );
    expect(reset.exists()).toBe(true);
    expect(reset.attributes("data-tip")).toBe("恢复默认工作目录");
    await reset.trigger("click");
    expect(store.newChatCwd).toBeNull();
  });

  it("会话进行中点击 cwd 在资源管理器中打开，不弹选择器、无 ×", async () => {
    store.currentThreadId = "t1";
    store.currentThreadCwd = "D:/thread";
    store.newChatCwd = "D:/custom"; // 会话态应忽略残留的新对话目录
    const wrapper = mountHeader();
    const chip = wrapper.find("button.brand-cwd");
    expect(chip.text()).toContain("D:/thread");
    expect(chip.attributes("data-tip")).toBe(
      "在资源管理器中打开：D:/thread",
    );
    expect(chip.classes()).not.toContain("pickable");
    expect(wrapper.find("button.brand-cwd-reset").exists()).toBe(false);
    mockedInvoke.mockResolvedValue(undefined);
    await chip.trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("open_url", {
      url: "D:/thread",
    });
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "pick_directory",
      expect.anything(),
    );
  });

});
