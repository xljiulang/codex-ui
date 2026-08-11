import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, newEmptyChat: vi.fn() };
});

import AppHeader from "../AppHeader.vue";
import { newEmptyChat, store } from "../../composables/useCodex";

const mockedNewChat = vi.mocked(newEmptyChat);

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
    const wrapper = mount(AppHeader);
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(store.showSettings).toBe(true);
    expect(store.showHistory).toBe(false);
  });

  it("历史记录按钮手动 toggle，设置面板互斥隐藏", async () => {
    const wrapper = mount(AppHeader);
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
    const wrapper = mount(AppHeader);
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(store.showSettings).toBe(false);
    expect(store.currentThreadId).toBe("t1");
    expect(mockedNewChat).not.toHaveBeenCalled();
  });

  it("历史面板打开时点设置进设置页，再点一次回到对话", async () => {
    store.showHistory = true;
    const wrapper = mount(AppHeader);
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(store.showSettings).toBe(true);
    expect(store.showHistory).toBe(false);
    await wrapper.find('button[aria-label="设置"]').trigger("click");
    expect(store.showSettings).toBe(false);
  });

  it("设置打开时点新建对话：关闭设置并新建", async () => {
    store.showSettings = true;
    const wrapper = mount(AppHeader);
    await wrapper.find('button[aria-label="新建对话"]').trigger("click");
    expect(store.showSettings).toBe(false);
    expect(mockedNewChat).toHaveBeenCalledTimes(1);
  });
});
