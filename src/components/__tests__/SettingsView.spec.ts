import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, saveSettings: vi.fn() };
});
vi.mock("../../composables/useSessionFs", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useSessionFs")>();
  return { ...mod, openPathInApp: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import SettingsView from "../SettingsView.vue";
import { saveSettings, settleConfirm, store } from "../../composables/useCodex";
import { openPathInApp } from "../../composables/useSessionFs";
import {
  activeTabId,
  openSettingsTab,
  SETTINGS_TAB_ID,
  tabs as _tabs,
} from "../../composables/useEditorTabs";
import { __resetTabsForTest } from "../../composables/useTabs";
import { __resetSessionTabsForTest } from "../../composables/useCodex/sessionState";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";
import type { SessionTab } from "../../composables/useCodex";

const tabs = _tabs as unknown as SessionTab[];

const mockedInvoke = vi.mocked(invoke);
const mockedSave = vi.mocked(saveSettings);
const mockedOpenPathInApp = vi.mocked(openPathInApp);

/** 打开设置标签并激活会话 s1（供「关闭/取消/保存」类测试使用） */
function mountWithSettingsTab() {
  tabs.push(makeSessionTab("s1", "t1"));
  activeTabId.value = "s1";
  openSettingsTab();
  return mount(SettingsView);
}

describe("SettingsView codex 可执行文件选择", () => {
  beforeEach(() => {
    __resetTabsForTest();
    store.settings.codex_path = "C:/tools/codex.exe";
    store.toast = "";
    mockedInvoke.mockReset();
    mockedSave.mockReset();
    mockedSave.mockResolvedValue(undefined);
  });

  it("渲染只读路径与选择按钮，路径行不再渲染文本输入框", () => {
    const wrapper = mount(SettingsView);
    const row = wrapper.find(".codex-path-row");
    expect(row.exists()).toBe(true);
    expect(row.find(".codex-path-value").text()).toContain("C:/tools/codex.exe");
    expect(row.find("button.codex-pick-btn").text()).toContain("选择文件");
    expect(row.find('input[type="text"]').exists()).toBe(false);
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

  it("清除路径时立即保存为 null", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find("button.codex-clear-btn").trigger("click");
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ codex_path: null }),
    );
  });

  it("未设置且检测到路径时展示当前使用说明", () => {
    store.settings.codex_path = null;
    store.server.codexPath = "C:/auto/codex.exe";
    const wrapper = mount(SettingsView);
    const note = wrapper.find(".setting-note");
    expect(note.exists()).toBe(true);
    expect(note.text()).toContain("当前使用（自动检测）");
    expect(note.text()).toContain("C:/auto/codex.exe");
  });

  it("未设置且未检测到路径时不显示当前使用说明", () => {
    store.settings.codex_path = null;
    store.server.codexPath = null;
    const wrapper = mount(SettingsView);
    expect(wrapper.find(".setting-note").exists()).toBe(false);
  });

  it("选择新路径后立即保存为自定义值，不显示检测说明", async () => {
    store.settings.codex_path = "C:/custom/codex.exe";
    store.server.codexPath = "C:/auto/codex.exe";
    mockedInvoke.mockResolvedValue("D:/new/codex.exe");
    const wrapper = mount(SettingsView);
    expect(wrapper.find(".codex-path-value").text()).toContain(
      "C:/custom/codex.exe",
    );
    expect(wrapper.find(".setting-note").exists()).toBe(false);

    await wrapper.find("button.codex-pick-btn").trigger("click");
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ codex_path: "D:/new/codex.exe" }),
    );
  });
});

describe("SettingsView 模型配置", () => {
  const sampleModelConfig = {
    config_path: "C:/apps/codex-ui/.codex/config.toml",
    config_exists: true,
    model: "deepseek-v4-flash",
    model_reasoning_effort: "high",
    model_provider: "codex-ui",
    forced_login_method: "api",
    model_catalog_json: "models.json",
    preferred_auth_method: "apikey",
    wire_api: "responses",
    name: "deepseek",
    base_url: "https://api.deepseek.com/",
    experimental_bearer_token: "你的 DeepSeek API Key",
    models_json_path: "C:/apps/codex-ui/.codex/models.json",
    models_json_exists: true,
    models_json: '{\n  "models": []\n}',
  };
  const sampleAgentsState = {
    agents_path: "C:/apps/codex-ui/.codex/AGENTS.md",
    exists: true,
    content: "# AGENTS.md\n\nWindows 环境。\n",
  };

  beforeEach(() => {
    store.toast = "";
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") return Promise.resolve(sampleModelConfig);
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    mockedSave.mockClear();
    mockedOpenPathInApp.mockReset();
    mockedOpenPathInApp.mockResolvedValue(true);
  });

  it("导航中「模型配置」位于「个性化」之后", () => {
    const wrapper = mount(SettingsView);
    const labels = wrapper
      .findAll(".settings-nav-item")
      .map((i) => i.text().trim());
    expect(labels.indexOf("个性化")).toBe(0);
    expect(labels.indexOf("模型配置")).toBe(1);
  });

  it("挂载时调用读取命令并填充三张卡片", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("model_config_read");
    expect(mockedInvoke).toHaveBeenCalledWith("custom_instructions_read");
    expect(
      (wrapper.find("input#model-config-model").element as HTMLInputElement)
        .value,
    ).toBe("deepseek-v4-flash");
    expect(
      (
        wrapper.find("input#model-config-effort").element as HTMLInputElement
      ).value,
    ).toBe("high");
    expect(
      (
        wrapper.find("input#model-config-base-url").element as HTMLInputElement
      ).value,
    ).toBe("https://api.deepseek.com/");
    expect(
      (
        wrapper.find("input#model-config-token").element as HTMLInputElement
      ).value,
    ).toBe("你的 DeepSeek API Key");
    expect(
      (
        wrapper.find("textarea.model-config-textarea")
          .element as HTMLTextAreaElement
      ).value,
    ).toBe('{\n  "models": []\n}');
    expect(
      (
        wrapper.find("textarea.custom-instructions-textarea")
          .element as HTMLTextAreaElement
      ).value,
    ).toBe("# AGENTS.md\n\nWindows 环境。\n");
    expect(wrapper.findAll(".model-config-card").length).toBe(3);
    expect(wrapper.find(".model-config-missing").exists()).toBe(false);
  });

  it("文件缺失时显示示例值与新建提示", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read")
        return Promise.resolve({
          ...sampleModelConfig,
          config_exists: false,
          models_json_exists: false,
          models_json: "",
        });
      if (cmd === "custom_instructions_read")
        return Promise.resolve({ ...sampleAgentsState, exists: false, content: "" });
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    expect(
      (wrapper.find("input#model-config-model").element as HTMLInputElement)
        .value,
    ).toBe("deepseek-v4-flash");
    const missing = wrapper.findAll(".model-config-missing");
    expect(missing.length).toBe(3);
    expect(missing[0].text()).toContain("文件不存在，保存时将新建");
    expect(wrapper.find(".model-config-path-link").exists()).toBe(false);
  });

  it("三张卡片保存按钮文案均为「保存」", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const saveButtons = wrapper.findAll(
      ".model-config-card .model-config-actions button.primary",
    );
    expect(saveButtons.map((b) => b.text().trim())).toEqual(["保存", "保存", "保存"]);
  });

  it("点 config.toml 卡「保存」调用 model_config_save 并提交 5 个可编辑值", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const cards = wrapper.findAll(".model-config-card");
    await cards[0].find(".model-config-actions button.primary").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("model_config_save", {
      input: {
        model: "deepseek-v4-flash",
        model_reasoning_effort: "high",
        name: "deepseek",
        base_url: "https://api.deepseek.com/",
        experimental_bearer_token: "你的 DeepSeek API Key",
      },
    });
    expect(store.toast).toContain("config.toml 已保存");
  });

  it("点 models.json 卡「保存」调用 models_json_save", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const cards = wrapper.findAll(".model-config-card");
    await cards[1].find(".model-config-actions button.primary").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("models_json_save", {
      content: '{\n  "models": []\n}',
    });
    expect(store.toast).toContain("models.json 已保存");
  });

  it("点 AGENTS.md 卡「保存」调用 custom_instructions_save", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const cards = wrapper.findAll(".model-config-card");
    await cards[2].find(".model-config-actions button.primary").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("custom_instructions_save", {
      content: "# AGENTS.md\n\nWindows 环境。\n",
    });
    expect(store.toast).toContain("AGENTS.md 已保存");
  });

  it("点击 config.toml 路径在应用内打开", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const links = wrapper.findAll(".model-config-path-link");
    expect(links.length).toBe(3);
    await links[0].trigger("click");
    await flushPromises();
    expect(mockedOpenPathInApp).toHaveBeenCalledWith(
      "C:/apps/codex-ui/.codex/config.toml",
    );
    expect(
      mockedInvoke.mock.calls.some(([name]) => name === "reveal_path"),
    ).toBe(false);
  });

  it("点击 models.json 路径在应用内打开", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const links = wrapper.findAll(".model-config-path-link");
    await links[1].trigger("click");
    await flushPromises();
    expect(mockedOpenPathInApp).toHaveBeenCalledWith(
      "C:/apps/codex-ui/.codex/models.json",
    );
  });

  it("点击 AGENTS.md 路径在应用内打开", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const links = wrapper.findAll(".model-config-path-link");
    await links[2].trigger("click");
    await flushPromises();
    expect(mockedOpenPathInApp).toHaveBeenCalledWith(
      "C:/apps/codex-ui/.codex/AGENTS.md",
    );
  });

  it("应用内打开失败时回退 reveal_path", async () => {
    mockedOpenPathInApp.mockResolvedValue(false);
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.findAll(".model-config-path-link")[0].trigger("click");
    await flushPromises();
    expect(mockedOpenPathInApp).toHaveBeenCalledWith(
      "C:/apps/codex-ui/.codex/config.toml",
    );
    expect(mockedInvoke).toHaveBeenCalledWith("reveal_path", {
      path: "C:/apps/codex-ui/.codex/config.toml",
    });
  });

  it("config.toml 刷新不重置 models.json 文本框", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .find("textarea.model-config-textarea")
      .setValue('{\n  "models": [],\n  "edited": true\n}');
    const readCallsBefore = mockedInvoke.mock.calls.filter(
      ([name]) => name === "model_config_read",
    ).length;
    const cards = wrapper.findAll(".model-config-card");
    await cards[0].find(".model-config-refresh-btn").trigger("click");
    await flushPromises();
    expect(
      mockedInvoke.mock.calls.filter(([name]) => name === "model_config_read")
        .length,
    ).toBe(readCallsBefore + 1);
    expect(
      (
        wrapper.find("textarea.model-config-textarea")
          .element as HTMLTextAreaElement
      ).value,
    ).toBe('{\n  "models": [],\n  "edited": true\n}');
  });

  it("models.json 与 AGENTS.md 刷新重新从磁盘读取", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const readCallsBefore = mockedInvoke.mock.calls.filter(
      ([name]) => name === "model_config_read",
    ).length;
    const agentsCallsBefore = mockedInvoke.mock.calls.filter(
      ([name]) => name === "custom_instructions_read",
    ).length;
    const cards = wrapper.findAll(".model-config-card");
    await cards[1].find(".model-config-refresh-btn").trigger("click");
    await flushPromises();
    await cards[2].find(".model-config-refresh-btn").trigger("click");
    await flushPromises();
    expect(
      mockedInvoke.mock.calls.filter(([name]) => name === "model_config_read")
        .length,
    ).toBe(readCallsBefore + 1);
    expect(
      mockedInvoke.mock.calls.filter(
        ([name]) => name === "custom_instructions_read",
      ).length,
    ).toBe(agentsCallsBefore + 1);
  });

  it("不再展示固定值 UI", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    expect(wrapper.find(".model-config-fixed").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("固定值");
  });

  it("读取失败时展示错误提示", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read")
        return Promise.reject(new Error("config.toml 解析失败: 语法错误"));
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    mount(SettingsView);
    await flushPromises();
    expect(store.toast).toContain("config.toml 解析失败");
  });
});

describe("SettingsView 设置标签行为", () => {
  let wrapper: ReturnType<typeof mount> | undefined;

  beforeEach(() => {
    __resetTabsForTest();
    __resetSessionTabsForTest();
    store.toast = "";
    mockedInvoke.mockReset();
    mockedSave.mockClear();
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
  });

  it("渲染为全宽页面而非模态框，面板内不再含标题与关闭按钮", () => {
    wrapper = mount(SettingsView);
    expect(wrapper.find(".settings-page").exists()).toBe(true);
    expect(wrapper.find(".modal-mask").exists()).toBe(false);
    expect(wrapper.find(".settings-page-title").exists()).toBe(false);
    expect(wrapper.find('button[aria-label="关闭设置"]').exists()).toBe(false);
    expect(wrapper.find(".settings-section").exists()).toBe(true);
  });

  it("按 Escape 不关闭设置标签", async () => {
    wrapper = mountWithSettingsTab();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wrapper.vm.$nextTick();
    expect(tabs.some((t) => t.id === SETTINGS_TAB_ID)).toBe(true);
  });

  it("个性化 / 模型配置 / 通用设置 / 插件管理四个分区均渲染", () => {
    wrapper = mount(SettingsView);
    const titles = wrapper.findAll(".settings-section-title").map((s) => s.text());
    expect(titles).toContain("个性化");
    expect(titles).toContain("模型配置");
    expect(titles).toContain("通用设置");
    expect(titles).toContain("插件管理");
  });

  it("左侧导航渲染四个分类，默认选中第一个", () => {
    wrapper = mount(SettingsView);
    const items = wrapper.findAll(".settings-nav-item");
    expect(items.map((i) => i.text().trim())).toEqual([
      "个性化",
      "模型配置",
      "通用设置",
      "插件管理",
    ]);
    expect(items[0].classes()).toContain("active");
    expect(items[1].classes()).not.toContain("active");
    expect(items[2].classes()).not.toContain("active");
    expect(items[3].classes()).not.toContain("active");
    const personal = wrapper
      .find(".settings-section-personalization")
      .element as HTMLElement;
    const general = wrapper.find(".settings-section-general").element as HTMLElement;
    const plugins = wrapper.find(".settings-section-plugins").element as HTMLElement;
    expect(personal.style.display).not.toBe("none");
    expect(general.style.display).toBe("none");
    expect(plugins.style.display).toBe("none");
  });

  it("点击「插件管理」切换右侧面板", async () => {
    wrapper = mount(SettingsView);
    const items = wrapper.findAll(".settings-nav-item");
    const pluginItem = items.find((i) => i.text().includes("插件管理"))!;
    await pluginItem.trigger("click");
    expect(items[0].classes()).not.toContain("active");
    expect(pluginItem.classes()).toContain("active");
    const general = wrapper.find(".settings-section-general").element as HTMLElement;
    const plugins = wrapper.find(".settings-section-plugins").element as HTMLElement;
    expect(general.style.display).toBe("none");
    expect(plugins.style.display).not.toBe("none");
  });

  it("记忆模式行合并：下拉与重置按钮同行，无说明文字", () => {
    wrapper = mount(SettingsView);
    const row = wrapper
      .findAll(".settings-section-general .setting-row")
      .find((r) => r.text().includes("记忆模式"))!;
    expect(row.exists()).toBe(true);
    expect(row.find("select.memory-mode-select").exists()).toBe(true);
    expect(row.find("button.memory-reset-btn").exists()).toBe(true);
    expect(row.find(".setting-value").exists()).toBe(false);
    expect(row.text()).not.toContain("清空全部已保存的记忆");
  });
});

describe("SettingsView 主题保存后生效", () => {
  beforeEach(() => {
    __resetTabsForTest();
    store.settings.theme = "blue";
    store.toast = "";
    mockedSave.mockClear();
    // 模拟真实 saveSettings：patch 立即合并进 store（即时保存语义）
    mockedSave.mockImplementation((patch) => {
      store.settings = { ...store.settings, ...patch };
      return Promise.resolve();
    });
    document.documentElement.dataset.theme = "blue";
  });

  it("点主题卡片立即保存并更新选中态", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find('.theme-card[data-theme-id="dark"]').trigger("click");
    await flushPromises();

    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ theme: "dark" }),
    );
    expect(store.settings.theme).toBe("dark");
    expect(
      wrapper.find('.theme-card[data-theme-id="dark"]').classes(),
    ).toContain("selected");
    expect(
      wrapper.find('.theme-card[data-theme-id="blue"]').classes(),
    ).not.toContain("selected");
  });

  it("点主题卡片即时预览到根节点并写入 store", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find('.theme-card[data-theme-id="light"]').trigger("click");
    await flushPromises();
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(store.settings.theme).toBe("light");
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ theme: "light" }),
    );
    wrapper.unmount();
  });

  it("保存成功后重开设置页：主题选中态为已保存值", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find('.theme-card[data-theme-id="dark"]').trigger("click");
    await flushPromises();
    wrapper.unmount();

    const reopened = mount(SettingsView);
    expect(
      reopened.find('.theme-card[data-theme-id="dark"]').classes(),
    ).toContain("selected");
    expect(
      reopened.find('.theme-card[data-theme-id="blue"]').classes(),
    ).not.toContain("selected");
    reopened.unmount();
  });

  it("保存失败时提示错误", async () => {
    mockedSave.mockRejectedValueOnce(new Error("写入配置失败"));
    const wrapper = mount(SettingsView);
    await wrapper.find('.theme-card[data-theme-id="dark"]').trigger("click");
    await flushPromises();
    expect(store.toast).toContain("写入配置失败");
    wrapper.unmount();
  });
});

describe("SettingsView 默认权限", () => {
  beforeEach(() => {
    __resetTabsForTest();
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

  it("切换默认权限后立即保存", async () => {
    const wrapper = mount(SettingsView);
    await wrapper
      .find("select.default-permission-select")
      .setValue("full-access");
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ default_permission: "full-access" }),
    );
    wrapper.unmount();
  });
});

describe("SettingsView 记忆管理", () => {
  let wrapper: ReturnType<typeof mount> | undefined;

  beforeEach(() => {
    __resetTabsForTest();
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

  it("切换记忆模式后立即保存", async () => {
    wrapper = mount(SettingsView);
    await wrapper.find("select.memory-mode-select").setValue("enabled");
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ memory_mode: "enabled" }),
    );
  });

  it("有当前会话时切换记忆模式后同步 thread/memoryMode/set", async () => {
    wrapper = mount(SettingsView);
    await wrapper.find("select.memory-mode-select").setValue("enabled");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/memoryMode/set",
      params: { threadId: "t1", mode: "enabled" },
    });
  });

  it("无当前会话时切换记忆模式不调用 thread/memoryMode/set", async () => {
    __resetSessionTabsForTest();
    wrapper = mount(SettingsView);
    await wrapper.find("select.memory-mode-select").setValue("enabled");
    await flushPromises();
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "codex_rpc",
      expect.objectContaining({ method: "thread/memoryMode/set" }),
    );
  });

  it("同步失败 toast 错误但设置仍已保存", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "thread/memoryMode/set") {
        throw new Error("同步记忆失败");
      }
      return {};
    });
    wrapper = mount(SettingsView);
    await wrapper.find("select.memory-mode-select").setValue("enabled");
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledTimes(1);
    expect(store.toast).toContain("同步记忆失败");
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

describe("SettingsView 终端 Shell", () => {
  let wrapper: ReturnType<typeof mount> | undefined;

  beforeEach(() => {
    __resetTabsForTest();
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
    store.settings.terminal_shell = "cmd";
    store.toast = "";
    store.confirm = null;
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue({});
    mockedSave.mockClear();
    mockedSave.mockResolvedValue(undefined);
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
  });

  it("渲染终端 Shell 下拉，默认选中 cmd", () => {
    wrapper = mount(SettingsView);
    const select = wrapper.find("select.terminal-shell-select");
    expect(select.exists()).toBe(true);
    expect((select.element as HTMLSelectElement).value).toBe("cmd");
    const options = wrapper
      .findAll("select.terminal-shell-select option")
      .map((o) => o.attributes("value"));
    expect(options).toEqual(["cmd", "powershell"]);
  });

  it("终端 Shell 行位于通用分区内第一项", () => {
    wrapper = mount(SettingsView);
    const section = wrapper.find(".settings-section-general");
    const labels = section
      .findAll(".settings .setting-row")
      .map((row) => row.find("label").text());
    expect(labels[0]).toBe("终端 Shell");
  });

  it("切换 PowerShell 后立即保存", async () => {
    wrapper = mount(SettingsView);
    await wrapper.find("select.terminal-shell-select").setValue("powershell");
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ terminal_shell: "powershell" }),
    );
  });
});

describe("SettingsView 插件管理", () => {
  let wrapper: ReturnType<typeof mount> | undefined;

  beforeEach(() => {
    __resetTabsForTest();
    __resetSessionTabsForTest();
    store.toast = "";
    store.confirm = null;
    mockedInvoke.mockReset();
    mockedSave.mockClear();
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
  });

  it("按市场分组渲染插件与状态标签，区分本地/远程目录", async () => {
    mockedInvoke.mockResolvedValue({
      marketplaces: [
        {
          name: "openai-bundled",
          path: "C:/x/bundled",
          plugins: [
            {
              id: "browser",
              name: "browser",
              installed: true,
              enabled: true,
              interface: { displayName: "Browser", shortDescription: "浏览器控制" },
            },
            {
              id: "pdf",
              name: "pdf",
              installed: false,
              enabled: false,
              interface: { displayName: "PDF" },
            },
          ],
        },
        {
          name: "openai-curated",
          path: null,
          plugins: [
            {
              id: "gmail",
              name: "gmail",
              installed: false,
              enabled: false,
              interface: { displayName: "Gmail" },
            },
          ],
        },
      ],
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    const mps = wrapper.findAll(".plugin-marketplace");
    expect(mps.length).toBe(2);
    expect(wrapper.text()).toContain("Browser");
    expect(wrapper.text()).toContain("PDF");
    expect(wrapper.text()).toContain("Gmail");
    expect(wrapper.text()).toContain("已启用");
    expect(wrapper.text()).toContain("未安装");
    expect(mps[0].text()).toContain("本地市场");
    expect(mps[1].text()).toContain("官方远程目录");
  });

  it("点击市场标题折叠/展开插件列表", async () => {
    mockedInvoke.mockResolvedValue({
      marketplaces: [
        {
          name: "openai-bundled",
          path: "C:/x/bundled",
          plugins: [
            {
              id: "browser",
              name: "browser",
              installed: true,
              enabled: true,
              interface: { displayName: "Browser" },
            },
          ],
        },
      ],
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    expect(wrapper.find(".plugin-list").exists()).toBe(true);
    const head = wrapper.find(".plugin-marketplace-head");
    await head.trigger("click");
    expect(wrapper.find(".plugin-list").exists()).toBe(false);
    expect(head.classes()).toContain("collapsed");
    await head.trigger("click");
    expect(wrapper.find(".plugin-list").exists()).toBe(true);
    expect(head.classes()).not.toContain("collapsed");
  });

  it("点击移除市场按钮不触发折叠", async () => {
    mockedInvoke.mockResolvedValue({
      marketplaces: [
        {
          name: "openai-bundled",
          path: "C:/x/bundled",
          plugins: [
            {
              id: "browser",
              name: "browser",
              installed: true,
              enabled: true,
              interface: { displayName: "Browser" },
            },
          ],
        },
      ],
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    const head = wrapper.find(".plugin-marketplace-head");
    await wrapper.find(".plugin-market-remove").trigger("click");
    expect(store.confirm).toBeTruthy();
    expect(head.classes()).not.toContain("collapsed");
    expect(wrapper.find(".plugin-list").exists()).toBe(true);
  });

  it("加载失败展示 marketplaceLoadErrors", async () => {
    mockedInvoke.mockResolvedValue({
      marketplaces: [],
      marketplaceLoadErrors: [{ name: "bad-repo", error: "git clone 失败" }],
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    expect(wrapper.find(".plugin-load-error").text()).toContain("git clone 失败");
  });

  it("本地市场插件安装传 marketplacePath", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "plugin/list") {
        return {
          marketplaces: [
            {
              name: "openai-bundled",
              path: "C:/x/bundled",
              plugins: [
                {
                  id: "pdf",
                  name: "pdf",
                  installed: false,
                  interface: { displayName: "PDF" },
                },
              ],
            },
          ],
        };
      }
      return {};
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-install-btn").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "plugin/install",
      params: { marketplacePath: "C:/x/bundled", pluginName: "pdf" },
    });
    expect(store.toast).toContain("已安装 PDF");
  });

  it("远程目录插件安装传 remoteMarketplaceName", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "plugin/list") {
        return {
          marketplaces: [
            {
              name: "openai-curated",
              path: null,
              plugins: [
                {
                  id: "gmail",
                  name: "gmail",
                  installed: false,
                  interface: { displayName: "Gmail" },
                },
              ],
            },
          ],
        };
      }
      return {};
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-install-btn").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "plugin/install",
      params: { remoteMarketplaceName: "openai-curated", pluginName: "gmail" },
    });
  });

  it("安装需认证时提示账号登录（API key 不可用）", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "plugin/list") {
        return {
          marketplaces: [
            {
              name: "openai-curated",
              path: null,
              plugins: [
                {
                  id: "gmail",
                  name: "gmail",
                  installed: false,
                  interface: { displayName: "Gmail" },
                },
              ],
            },
          ],
        };
      }
      if (cmd === "codex_rpc" && args?.method === "plugin/install") {
        throw new Error("authentication required");
      }
      return {};
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-install-btn").trigger("click");
    await flushPromises();
    expect(store.toast).toContain("需要账号登录");
  });

  it("卸载需确认，确认后调用 plugin/uninstall", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "plugin/list") {
        return {
          marketplaces: [
            {
              name: "openai-bundled",
              path: "C:/x/bundled",
              plugins: [
                {
                  id: "browser",
                  name: "browser",
                  installed: true,
                  enabled: true,
                  interface: { displayName: "Browser" },
                },
              ],
            },
          ],
        };
      }
      return {};
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-uninstall-btn").trigger("click");
    expect(store.confirm).toBeTruthy();
    settleConfirm(true);
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "plugin/uninstall",
      params: { pluginId: "browser" },
    });
  });

  it("添加市场调用 marketplace/add 并刷新目录", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "marketplace/add") {
        return {};
      }
      if (cmd === "codex_rpc" && args?.method === "plugin/list") {
        return { marketplaces: [] };
      }
      return {};
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    expect(
      wrapper.find(".plugin-market-add input").attributes("placeholder"),
    ).toBe("Git URL 或本地绝对路径");
    await wrapper
      .find(".plugin-market-add input")
      .setValue("owner/repo");
    await wrapper.find(".plugin-market-add .btn").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "marketplace/add",
      params: { source: "owner/repo" },
    });
    expect(store.toast).toContain("市场已添加");
  });

  it("管理员禁用插件不显示安装按钮", async () => {
    mockedInvoke.mockResolvedValue({
      marketplaces: [
        {
          name: "openai-bundled",
          path: "C:/x/bundled",
          plugins: [
            {
              id: "banned",
              name: "banned",
              installed: false,
              enabled: false,
              availability: "DisabledByAdmin",
              disabledReason: "disabled by admin",
              interface: { displayName: "Banned" },
            },
          ],
        },
      ],
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    expect(wrapper.text()).toContain("管理员已禁用");
    expect(wrapper.find(".plugin-install-btn").attributes("disabled")).toBeDefined();
  });
});
