import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import ConfirmDialog from "../ConfirmDialog.vue";
import { store } from "../../composables/useCodex";

describe("ConfirmDialog 全局确认框", () => {
  let wrapper: ReturnType<typeof mount> | undefined;

  beforeEach(() => {
    store.confirm = null;
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
  });

  function setConfirm(resolve: (ok: boolean) => void) {
    store.confirm = {
      title: "切换会话",
      message: "当前会话仍在进行中，切换将停止当前回合。是否继续？",
      confirmLabel: "停止并切换",
      cancelLabel: "取消",
      resolve,
    };
  }

  it("无确认请求时不渲染", () => {
    wrapper = mount(ConfirmDialog);
    expect(wrapper.find(".modal-mask").exists()).toBe(false);
  });

  it("渲染标题、正文与按钮", () => {
    setConfirm(vi.fn());
    wrapper = mount(ConfirmDialog);
    expect(wrapper.find(".modal-title").text()).toContain("切换会话");
    expect(wrapper.text()).toContain("当前会话仍在进行中");
    expect(wrapper.text()).toContain("停止并切换");
    expect(wrapper.text()).toContain("取消");
  });

  it("点「停止并切换」resolve(true) 并关闭", async () => {
    const resolve = vi.fn();
    setConfirm(resolve);
    wrapper = mount(ConfirmDialog);
    const btn = wrapper
      .findAll(".modal-foot .btn")
      .find((b) => b.text().trim() === "停止并切换");
    await btn!.trigger("click");
    expect(resolve).toHaveBeenCalledWith(true);
    expect(store.confirm).toBeNull();
  });

  it("点「取消」resolve(false) 并关闭", async () => {
    const resolve = vi.fn();
    setConfirm(resolve);
    wrapper = mount(ConfirmDialog);
    const btn = wrapper
      .findAll(".modal-foot .btn")
      .find((b) => b.text().trim() === "取消");
    await btn!.trigger("click");
    expect(resolve).toHaveBeenCalledWith(false);
    expect(store.confirm).toBeNull();
  });

  it("按 Escape resolve(false) 并关闭", async () => {
    const resolve = vi.fn();
    setConfirm(resolve);
    wrapper = mount(ConfirmDialog);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wrapper.vm.$nextTick();
    expect(resolve).toHaveBeenCalledWith(false);
    expect(store.confirm).toBeNull();
  });
});
