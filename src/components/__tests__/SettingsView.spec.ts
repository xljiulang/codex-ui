import { describe, expect, it, vi, beforeEach } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, saveSettings: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import SettingsView from "../SettingsView.vue";
import { saveSettings, store } from "../../composables/useCodex";

const mockedInvoke = vi.mocked(invoke);
const mockedSave = vi.mocked(saveSettings);

describe("SettingsView codex 可执行文件选择", () => {
  beforeEach(() => {
    store.settings.codex_path = "C:/tools/codex.exe";
    store.toast = "";
    mockedInvoke.mockReset();
    mockedSave.mockReset();
    mockedSave.mockResolvedValue(undefined);
  });

  it("渲染只读路径与选择按钮，不再渲染文本输入框", () => {
    const wrapper = mount(SettingsView);
    const row = wrapper.find(".setting-path-row");
    expect(row.exists()).toBe(true);
    expect(row.find(".setting-value").text()).toContain("C:/tools/codex.exe");
    expect(row.find("button.btn").text()).toContain("选择文件");
    expect(wrapper.find('input[type="text"]').exists()).toBe(false);
  });

  it("空路径时显示未设置提示，无清除按钮", () => {
    store.settings.codex_path = null;
    const wrapper = mount(SettingsView);
    expect(wrapper.find(".setting-value").text()).toContain("未设置");
    expect(wrapper.find("button.btn.danger").exists()).toBe(false);
  });

  it("点选择文件以当前路径父目录为 initialDir，选中后更新展示", async () => {
    mockedInvoke.mockResolvedValue("D:/apps/codex/codex.exe");
    const wrapper = mount(SettingsView);
    await wrapper.find("button.btn").trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("pick_codex_file", {
      initialDir: "C:/tools",
    });
    expect(wrapper.find(".setting-value").text()).toContain(
      "D:/apps/codex/codex.exe",
    );
  });

  it("选择失败时展示错误提示", async () => {
    mockedInvoke.mockRejectedValue(new Error("请选择名为 codex.exe 的文件"));
    const wrapper = mount(SettingsView);
    await wrapper.find("button.btn").trigger("click");
    await flushPromises();
    expect(store.toast).toContain("请选择名为 codex.exe 的文件");
  });

  it("清除按钮清空路径并隐藏自身", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find("button.btn.danger").trigger("click");
    expect(wrapper.find(".setting-value").text()).toContain("未设置");
    expect(wrapper.find("button.btn.danger").exists()).toBe(false);
  });

  it("保存设置时空路径存为 null", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find("button.btn.danger").trigger("click");
    await wrapper.find("button.btn.primary").trigger("click");
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ codex_path: null }),
    );
  });
});
