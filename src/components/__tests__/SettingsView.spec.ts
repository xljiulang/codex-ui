import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, saveSettings: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import SettingsView from "../SettingsView.vue";
import { saveSettings, settleConfirm, store } from "../../composables/useCodex";
import { activeTabId, tabs as _tabs } from "../../composables/useEditorTabs";
import { __resetSessionTabsForTest } from "../../composables/useCodex/sessionState";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";
import type { SessionTab } from "../../composables/useCodex";

const tabs = _tabs as unknown as SessionTab[];

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
    const row = wrapper.find(".codex-path-row");
    expect(row.exists()).toBe(true);
    expect(row.find(".codex-path-value").text()).toContain("C:/tools/codex.exe");
    expect(row.find("button.codex-pick-btn").text()).toContain("选择文件");
    expect(wrapper.find('input[type="text"]').exists()).toBe(false);
  });

  it("空路径时显示未设置提示，无清除按钮", () => {
    store.settings.codex_path = null;
    store.server.codexPath = null;
    const wrapper = mount(SettingsView);
    expect(wrapper.find(".codex-path-value").text()).toContain(
      "未设置（自动查找）",
    );
    expect(wrapper.find("button.codex-clear-btn").exists()).toBe(false);
  });

  it("点选择文件以当前路径父目录为 initialDir，选中后更新展示", async () => {
    mockedInvoke.mockResolvedValue("D:/apps/codex/codex.exe");
    const wrapper = mount(SettingsView);
    await wrapper.find("button.codex-pick-btn").trigger("click");
    expect(mockedInvoke).toHaveBeenCalledWith("pick_codex_file", {
      initialDir: "C:/tools",
    });
    expect(wrapper.find(".codex-path-value").text()).toContain(
      "D:/apps/codex/codex.exe",
    );
  });

  it("选择失败时展示错误提示", async () => {
    mockedInvoke.mockRejectedValue(new Error("请选择名为 codex.exe 的文件"));
    const wrapper = mount(SettingsView);
    await wrapper.find("button.codex-pick-btn").trigger("click");
    await flushPromises();
    expect(store.toast).toContain("请选择名为 codex.exe 的文件");
  });

  it("清除按钮清空路径并隐藏自身", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find("button.codex-clear-btn").trigger("click");
    expect(wrapper.find(".codex-path-value").text()).toContain("未设置");
    expect(wrapper.find("button.codex-clear-btn").exists()).toBe(false);
  });

  it("保存设置时空路径存为 null", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find("button.codex-clear-btn").trigger("click");
    await wrapper.find("button.btn.primary").trigger("click");
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ codex_path: null }),
    );
  });

  it("未设置且检测到路径时展示当前使用说明，保存不写入该路径", async () => {
    store.settings.codex_path = null;
    store.server.codexPath = "C:/auto/codex.exe";
    const wrapper = mount(SettingsView);
    const note = wrapper.find(".setting-note");
    expect(note.exists()).toBe(true);
    expect(note.text()).toContain("当前使用（自动检测）");
    expect(note.text()).toContain("C:/auto/codex.exe");

    await wrapper.find("button.btn.primary").trigger("click");
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ codex_path: null }),
    );
  });

  it("未设置且未检测到路径时不显示当前使用说明", () => {
    store.settings.codex_path = null;
    store.server.codexPath = null;
    const wrapper = mount(SettingsView);
    expect(wrapper.find(".setting-note").exists()).toBe(false);
  });

  it("设置自定义路径时保存为自定义值，不显示检测说明", async () => {
    store.settings.codex_path = "C:/custom/codex.exe";
    store.server.codexPath = "C:/auto/codex.exe";
    const wrapper = mount(SettingsView);
    expect(wrapper.find(".codex-path-value").text()).toContain(
      "C:/custom/codex.exe",
    );
    expect(wrapper.find(".setting-note").exists()).toBe(false);

    await wrapper.find("button.btn.primary").trigger("click");
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ codex_path: "C:/custom/codex.exe" }),
    );
  });
});

describe("SettingsView 模态框行为", () => {
  let wrapper: ReturnType<typeof mount> | undefined;

  beforeEach(() => {
    store.showSettings = true;
    store.toast = "";
    mockedSave.mockClear();
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
  });

  it("渲染模态结构：遮罩、对话框、标题与关闭按钮", () => {
    wrapper = mount(SettingsView);
    expect(wrapper.find(".modal-mask").exists()).toBe(true);
    expect(wrapper.find(".modal.settings-modal").exists()).toBe(true);
    expect(wrapper.find(".modal-title").text()).toContain("设置");
    expect(wrapper.find('button[aria-label="关闭设置"]').exists()).toBe(true);
    expect(wrapper.find(".modal-body .settings").exists()).toBe(true);
  });

  it("按 Escape 关闭设置", async () => {
    wrapper = mount(SettingsView);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wrapper.vm.$nextTick();
    expect(store.showSettings).toBe(false);
  });

  it("点击关闭 X 关闭设置", async () => {
    wrapper = mount(SettingsView);
    await wrapper.find('button[aria-label="关闭设置"]').trigger("click");
    expect(store.showSettings).toBe(false);
  });

  it("点击「取消」关闭设置且不保存", async () => {
    wrapper = mount(SettingsView);
    const cancel = wrapper
      .findAll(".modal-foot .btn")
      .find((b) => b.text().trim() === "取消");
    expect(cancel).toBeDefined();
    await cancel!.trigger("click");
    expect(store.showSettings).toBe(false);
    expect(mockedSave).not.toHaveBeenCalled();
  });

  it("点击「保存」成功后关闭设置并提示已保存", async () => {
    wrapper = mount(SettingsView);
    await wrapper.find("button.btn.primary").trigger("click");
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledTimes(1);
    expect(store.showSettings).toBe(false);
    expect(store.toast).toContain("设置已保存");
  });

  it("点击遮罩不关闭（防误触丢改动）", async () => {
    wrapper = mount(SettingsView);
    await wrapper.find(".modal-mask").trigger("click");
    expect(store.showSettings).toBe(true);
  });
});

describe("SettingsView 主题保存后生效", () => {
  beforeEach(() => {
    store.settings.theme = "blue";
    store.toast = "";
    mockedSave.mockClear();
    document.documentElement.dataset.theme = "blue";
  });

  it("点主题卡片只更新选中态，不即时保存或修改 store", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find('.theme-card[data-theme-id="dark"]').trigger("click");

    expect(mockedSave).not.toHaveBeenCalled();
    expect(store.settings.theme).toBe("blue");
    expect(
      wrapper.find('.theme-card[data-theme-id="dark"]').classes(),
    ).toContain("selected");
    expect(
      wrapper.find('.theme-card[data-theme-id="blue"]').classes(),
    ).not.toContain("selected");
  });

  it("选主题后点保存：saveSettings 收到新主题并关闭", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find('.theme-card[data-theme-id="light"]').trigger("click");
    await wrapper.find("button.btn.primary").trigger("click");
    await flushPromises();

    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ theme: "light" }),
    );
    expect(store.showSettings).toBe(false);
  });

  it("未保存取消后重开：主题选中态回到已保存值", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find('.theme-card[data-theme-id="dark"]').trigger("click");
    wrapper.unmount();

    const reopened = mount(SettingsView);
    expect(
      reopened.find('.theme-card[data-theme-id="dark"]').classes(),
    ).not.toContain("selected");
    expect(
      reopened.find('.theme-card[data-theme-id="blue"]').classes(),
    ).toContain("selected");
    reopened.unmount();
  });

  it("选主题卡片即时预览到根节点，但不写入 store", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find('.theme-card[data-theme-id="dark"]').trigger("click");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(store.settings.theme).toBe("blue");
    expect(mockedSave).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("未保存取消后还原根节点主题", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find('.theme-card[data-theme-id="dark"]').trigger("click");
    expect(document.documentElement.dataset.theme).toBe("dark");
    const cancel = wrapper
      .findAll(".modal-foot .btn")
      .find((b) => b.text().trim() === "取消");
    await cancel!.trigger("click");
    expect(store.showSettings).toBe(false);
    expect(document.documentElement.dataset.theme).toBe("blue");
    wrapper.unmount();
  });

  it("保存后保留预览主题不回退", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find('.theme-card[data-theme-id="light"]').trigger("click");
    expect(document.documentElement.dataset.theme).toBe("light");
    await wrapper.find("button.btn.primary").trigger("click");
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ theme: "light" }),
    );
    expect(document.documentElement.dataset.theme).toBe("light");
    wrapper.unmount();
  });
});

describe("SettingsView 默认权限", () => {
  beforeEach(() => {
    store.settings.default_permission = "ask-for-approval";
    store.toast = "";
    mockedSave.mockClear();
  });

  it("渲染默认权限下拉并选中已保存值", () => {
    const wrapper = mount(SettingsView);
    const select = wrapper.find("select.default-permission-select");
    expect(select.exists()).toBe(true);
    expect((select.element as HTMLSelectElement).value).toBe("ask-for-approval");
    const labels = wrapper
      .findAll("select.default-permission-select option")
      .map((o) => o.text());
    expect(labels).toEqual(
      expect.arrayContaining(["请求批准", "帮我批准", "完全访问权限"]),
    );
    wrapper.unmount();
  });

  it("保存时 patch 包含所选默认权限", async () => {
    const wrapper = mount(SettingsView);
    await wrapper
      .find("select.default-permission-select")
      .setValue("full-access");
    await wrapper.find("button.btn.primary").trigger("click");
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ default_permission: "full-access" }),
    );
    wrapper.unmount();
  });

  it("取消不保存默认权限", async () => {
    const wrapper = mount(SettingsView);
    await wrapper
      .find("select.default-permission-select")
      .setValue("help-me-approve");
    const cancel = wrapper
      .findAll(".modal-foot .btn")
      .find((b) => b.text().trim() === "取消");
    expect(cancel).toBeDefined();
    await cancel!.trigger("click");
    expect(mockedSave).not.toHaveBeenCalled();
    wrapper.unmount();
  });
});

describe("SettingsView 记忆管理", () => {
  let wrapper: ReturnType<typeof mount> | undefined;

  beforeEach(() => {
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    store.settings.memory_mode = "disabled";
    store.toast = "";
    store.confirm = null;
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue({});
    mockedSave.mockClear();
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
  });

  it("渲染记忆模式下拉并选中已保存值", () => {
    store.settings.memory_mode = "enabled";
    wrapper = mount(SettingsView);
    const select = wrapper.find("select.memory-mode-select");
    expect(select.exists()).toBe(true);
    expect((select.element as HTMLSelectElement).value).toBe("enabled");
    const labels = wrapper
      .findAll("select.memory-mode-select option")
      .map((o) => o.text());
    expect(labels).toEqual(["关闭", "启用"]);
    wrapper.unmount();
  });

  it("保存时 patch 包含记忆模式", async () => {
    wrapper = mount(SettingsView);
    await wrapper.find("select.memory-mode-select").setValue("enabled");
    await wrapper.find("button.btn.primary").trigger("click");
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ memory_mode: "enabled" }),
    );
  });

  it("有当前会话时保存后立即同步 thread/memoryMode/set", async () => {
    wrapper = mount(SettingsView);
    await wrapper.find("select.memory-mode-select").setValue("enabled");
    await wrapper.find("button.btn.primary").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/memoryMode/set",
      params: { threadId: "t1", mode: "enabled" },
    });
  });

  it("无当前会话时保存不调用 thread/memoryMode/set", async () => {
    __resetSessionTabsForTest();
    wrapper = mount(SettingsView);
    await wrapper.find("button.btn.primary").trigger("click");
    await flushPromises();
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "codex_rpc",
      expect.objectContaining({ method: "thread/memoryMode/set" }),
    );
  });

  it("同步失败 toast 错误但设置仍保存并关闭", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "thread/memoryMode/set") {
        throw new Error("同步记忆失败");
      }
      return {};
    });
    wrapper = mount(SettingsView);
    await wrapper.find("button.btn.primary").trigger("click");
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledTimes(1);
    expect(store.toast).toContain("同步记忆失败");
    expect(store.showSettings).toBe(false);
  });

  it("重置记忆需确认，确认后调用 memory/reset 并提示", async () => {
    wrapper = mount(SettingsView);
    await wrapper.find("button.memory-reset-btn").trigger("click");
    expect(store.confirm).toBeTruthy();
    expect(mockedInvoke).not.toHaveBeenCalledWith("codex_rpc", {
      method: "memory/reset",
      params: null,
    });
    settleConfirm(true);
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "memory/reset",
      params: null,
    });
    expect(store.toast).toContain("记忆已重置");
  });

  it("重置记忆取消时不调用 memory/reset", async () => {
    wrapper = mount(SettingsView);
    await wrapper.find("button.memory-reset-btn").trigger("click");
    settleConfirm(false);
    await flushPromises();
    expect(mockedInvoke).not.toHaveBeenCalledWith("codex_rpc", {
      method: "memory/reset",
      params: null,
    });
  });
});
