import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";
import { config, flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
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
import { tooltipDirective } from "../../directives/tooltip";

// SettingsView 大量使用 v-tooltip，统一注入该指令避免逐 mount 配置
config.global.directives = { tooltip: tooltipDirective };

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
    // 默认返回 undefined：验证 composable 的空快照守卫不会清掉已有状态
    (mockedInvoke as ReturnType<typeof vi.fn>).mockImplementation(async () => undefined);
  });

  it("渲染只读路径与选择按钮，路径行不再渲染文本输入框", () => {
    const wrapper = mount(SettingsView);
    const row = wrapper.find(".codex-path-row");
    expect(row.exists()).toBe(true);
    expect(row.find(".codex-path-value").text()).toContain("C:/tools/codex.exe");
    expect(row.find("button.codex-pick-btn").attributes("data-tip")).toBe("选择文件");
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
    config_content: 'model = "deepseek-v4-flash"\nmodel_reasoning_effort = "high"\n',
    model_catalog_json: "",
    model_catalog_path: "C:/apps/codex-ui/.codex/models.json",
    model_catalog_exists: true,
    model_catalog: '{\n  "models": []\n}',
    model: "deepseek-v4-flash",
    model_reasoning_effort: "high",
    model_provider: "deepseek",
    preferred_auth_method: "apikey",
    forced_login_method: "api",
    openai_api_key_present: false,
    providers: [
      {
        key: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/",
        env_key: "",
        experimental_bearer_token: "sk-test",
        wire_api: "responses",
      },
      {
        key: "other",
        name: "Other",
        base_url: "https://other.example.com/v1",
        env_key: "OTHER_API_KEY",
        experimental_bearer_token: "",
        wire_api: "chat",
      },
    ],
  };
  /** config/read 返回：用户层原始 [model_providers.*] 与顶层标量（与真实 app-server 形状一致） */
  const sampleModelConfigRead = {
    config: {},
    layers: [
      {
        name: { type: "user", file: "C:/apps/codex-ui/.codex/config.toml", profile: null },
        version: "sha256:abc",
        config: {
          model: "deepseek-v4-flash",
          model_reasoning_effort: "high",
          model_provider: "deepseek",
          preferred_auth_method: "apikey",
          forced_login_method: "api",
          model_catalog_json: "",
          model_providers: {
            deepseek: {
              name: "DeepSeek",
              base_url: "https://api.deepseek.com/",
              env_key: "",
              experimental_bearer_token: "sk-test",
              wire_api: "responses",
            },
            other: {
              name: "Other",
              base_url: "https://other.example.com/v1",
              env_key: "OTHER_API_KEY",
              experimental_bearer_token: "",
              wire_api: "chat",
            },
          },
        },
        disabledReason: null,
      },
    ],
  };
  const sampleAgentsState = {
    agents_path: "C:/apps/codex-ui/.codex/AGENTS.md",
    exists: true,
    content: "# AGENTS.md\n\nWindows 环境。\n",
  };

  beforeEach(() => {
    store.toast = "";
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        return Promise.resolve(sampleModelConfigRead);
      }
      if (cmd === "model_config_read") return Promise.resolve(sampleModelConfig);
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    mockedSave.mockClear();
    mockedOpenPathInApp.mockReset();
    mockedOpenPathInApp.mockResolvedValue(true);
  });

  it("导航顺序：个性化 → 通用设置 → 模型配置", () => {
    const wrapper = mount(SettingsView);
    const labels = wrapper
      .findAll(".settings-nav-item")
      .map((i) => i.text().trim());
    expect(labels.indexOf("个性化")).toBe(0);
    expect(labels.indexOf("通用设置")).toBe(1);
    expect(labels.indexOf("模型配置")).toBe(2);
  });

  it("挂载时调用读取命令并填充三张卡片", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("model_config_read");
    expect(mockedInvoke).toHaveBeenCalledWith("custom_instructions_read");
    expect(
      (
        wrapper.findAll("textarea.model-config-textarea")[0]
          .element as HTMLTextAreaElement
      ).value,
    ).toBe('{\n  "models": []\n}');
    expect(
      (
        wrapper.find("textarea.custom-instructions-textarea")
          .element as HTMLTextAreaElement
      ).value,
    ).toBe("# AGENTS.md\n\nWindows 环境。\n");
    expect(wrapper.text()).toContain("模型提供方");
    expect(wrapper.text()).toContain("model_catalog_json");
    expect(wrapper.text()).toContain("AGENTS");
    expect(
      wrapper.findAll(".settings-section-model-config .model-config-card").length,
    ).toBe(3);
    expect(wrapper.find(".model-config-missing").exists()).toBe(false);
  });

  it("模型提供方列表排在 model 等输入之前", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const listEl = wrapper.element.querySelector(".model-providers-list");
    const modelEl = wrapper.element.querySelector("#model-config-ui-model");
    expect(listEl).toBeTruthy();
    expect(modelEl).toBeTruthy();
    const pos = listEl!.compareDocumentPosition(modelEl!);
    expect(pos & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("文件缺失时读取自动创建：卡片均可编辑并显示路径链接", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read")
        return Promise.resolve({
          ...sampleModelConfig,
          config_exists: true,
          config_content: "",
          model_catalog_exists: true,
          model_catalog: '{"models":[]}',
        });
      if (cmd === "custom_instructions_read")
        return Promise.resolve({ ...sampleAgentsState, exists: true, content: "" });
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    const textareas = wrapper.findAll("textarea.model-config-textarea");
    // 文件已自动创建：model_catalog_json 与 AGENTS 均可编辑
    expect(textareas[0].attributes("disabled")).toBeUndefined();
    expect(
      wrapper.find("textarea.custom-instructions-textarea").attributes("disabled"),
    ).toBeUndefined();
    expect(wrapper.find(".model-config-missing").exists()).toBe(false);
    // 两条路径链接均可用（model_catalog_json / AGENTS）
    expect(wrapper.findAll(".model-config-path-link").length).toBe(2);
  });

  it("三张卡片保存按钮标题均为「保存」", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const saveButtons = wrapper.findAll(
      ".settings-section-model-config .model-config-card .model-config-actions .model-config-save-btn",
    );
    expect(saveButtons.map((b) => b.attributes("data-tip"))).toEqual([
      "保存",
      "保存",
      "保存",
    ]);
  });

  it("点 model_catalog_json 卡「保存」调用 model_catalog_save", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const cards = wrapper.findAll(".model-config-card");
    await cards[1].find(".model-config-actions button.primary").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("model_catalog_save", {
      content: '{\n  "models": []\n}',
    });
    expect(store.toast).toContain("model_catalog_json 已保存");
  });

  it("点 AGENTS 卡「保存」调用 custom_instructions_save", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const cards = wrapper.findAll(".model-config-card");
    await cards[2].find(".model-config-actions button.primary").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("custom_instructions_save", {
      content: "# AGENTS.md\n\nWindows 环境。\n",
    });
    expect(store.toast).toContain("AGENTS 已保存");
  });

  it("点击 model_catalog_json 路径链接在应用内打开", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const links = wrapper.findAll(".model-config-path-link");
    await links[0].trigger("click");
    await flushPromises();
    expect(mockedOpenPathInApp).toHaveBeenCalledWith(
      "C:/apps/codex-ui/.codex/models.json",
    );
  });

  it("点击 AGENTS 路径链接在应用内打开", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const links = wrapper.findAll(".model-config-path-link");
    await links[1].trigger("click");
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
      "C:/apps/codex-ui/.codex/models.json",
    );
    expect(mockedInvoke).toHaveBeenCalledWith("reveal_path", {
      path: "C:/apps/codex-ui/.codex/models.json",
    });
  });

  it("渲染 DeepSeek 接入文档链接", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const link = wrapper.find(".model-config-docs-link");
    expect(link.exists()).toBe(true);
    expect(link.text()).toBe("DeepSeek 接入文档");
    expect(
      wrapper.find(".model-config-head-actions .model-config-docs-link").exists(),
    ).toBe(true);
  });

  it("点击 DeepSeek 接入文档链接用默认浏览器打开文档", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".model-config-docs-link").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("open_url", {
      url: "https://api-docs.deepseek.com/zh-cn/quick_start/agent_integrations/codex/",
    });
  });

  it("模型提供方重读不重置 model_catalog_json 文本框", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll("textarea.model-config-textarea")[0]
      .setValue('{\n  "models": [],\n  "edited": true\n}');
    const readCallsBefore = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "config/read",
    ).length;
    const cards = wrapper.findAll(".model-config-card");
    await cards[0].find(".model-config-reload-btn").trigger("click");
    await flushPromises();
    expect(
      mockedInvoke.mock.calls.filter(
        ([cmd, args]) =>
          cmd === "codex_rpc" &&
          (args as { method?: string } | undefined)?.method === "config/read",
      ).length,
    ).toBe(readCallsBefore + 1);
    expect(
      (
        wrapper.findAll("textarea.model-config-textarea")[0]
          .element as HTMLTextAreaElement
      ).value,
    ).toBe('{\n  "models": [],\n  "edited": true\n}');
  });

  it("model_catalog_json 与 AGENTS 刷新重新从磁盘读取", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const readCallsBefore = mockedInvoke.mock.calls.filter(
      ([name]) => name === "model_config_read",
    ).length;
    const agentsCallsBefore = mockedInvoke.mock.calls.filter(
      ([name]) => name === "custom_instructions_read",
    ).length;
    const cards = wrapper.findAll(".model-config-card");
    await cards[1].find(".model-config-reload-btn").trigger("click");
    await flushPromises();
    await cards[2].find(".model-config-reload-btn").trigger("click");
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

  it("保存模型提供方成功后自动重读 model_catalog_json 卡片", async () => {
    let catalogContent = '{\n  "models": []\n}';
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        return Promise.resolve(sampleModelConfigRead);
      }
      if (cmd === "model_config_read")
        return Promise.resolve({
          ...sampleModelConfig,
          model_catalog: catalogContent,
        });
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    const readCallsBefore = mockedInvoke.mock.calls.filter(
      ([name]) => name === "model_config_read",
    ).length;
    catalogContent = '{\n  "models": [{ "id": "gpt-x" }]\n}';
    await wrapper
      .findAll(".model-config-card")[0]
      .find(".model-config-actions button.primary")
      .trigger("click");
    await flushPromises();
    expect(
      mockedInvoke.mock.calls.filter(([name]) => name === "model_config_read")
        .length,
    ).toBe(readCallsBefore + 2);
    expect(
      (
        wrapper.findAll("textarea.model-config-textarea")[0]
          .element as HTMLTextAreaElement
      ).value,
    ).toBe('{\n  "models": [{ "id": "gpt-x" }]\n}');
  });

  it("model_catalog_json 未配置时禁用编辑", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read")
        return Promise.resolve({
          ...sampleModelConfig,
          model_catalog_path: "",
          model_catalog_exists: false,
          model_catalog: "",
        });
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    const textareas = wrapper.findAll("textarea.model-config-textarea");
    expect(textareas[0].attributes("disabled")).toBeDefined();
    expect(
      wrapper
        .findAll(".model-config-card")[1]
        .find(".model-config-actions button.primary")
        .attributes("disabled"),
    ).toBeDefined();
    expect(wrapper.text()).toContain("config 未配置 model_catalog_json");
    expect(wrapper.findAll(".model-config-path-link").length).toBe(1);
  });

  it("渲染提供方列表与激活单选", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const rows = wrapper.findAll(".model-provider-row");
    expect(rows.length).toBe(2);
    expect(rows[0].text()).toContain("DeepSeek");
    expect(rows[0].text()).toContain("deepseek");
    const radios = wrapper.findAll('input[name="model-provider-active"]');
    expect(radios.length).toBe(2);
    expect((radios[0].element as HTMLInputElement).checked).toBe(true);
    expect((radios[1].element as HTMLInputElement).checked).toBe(false);
  });

  it("空列表时添加首个提供方自动激活", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read")
        return Promise.resolve({
          ...sampleModelConfig,
          model: "",
          model_reasoning_effort: "",
          model_provider: "",
          providers: [],
        });
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    expect(wrapper.text()).toContain("还没有提供方");
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".model-provider-form").exists()).toBe(true);
    await wrapper
      .find('input[placeholder="如 my-provider"]')
      .setValue("first");
    await wrapper
      .find('input[placeholder="如 DeepSeek"]')
      .setValue("First");
    await wrapper
      .find('input[placeholder="https://api.example.com/v1"]')
      .setValue("https://first.example.com/v1");
    await wrapper
      .find('input[placeholder="如 OPENAI_API_KEY"]')
      .setValue("MY_API_KEY");
    await wrapper.find(".provider-form-submit").trigger("click");
    await flushPromises();
    const radios = wrapper.findAll('input[name="model-provider-active"]');
    expect(radios.length).toBe(1);
    expect((radios[0].element as HTMLInputElement).checked).toBe(true);
  });

  it("重复提供方标识被拦截", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    await wrapper.find('input[placeholder="如 my-provider"]').setValue("deepseek");
    await wrapper.find(".provider-form-submit").trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("提供方标识已存在");
    expect(wrapper.findAll(".model-provider-row").length).toBe(2);
  });

  it("新增提供方以弹窗呈现：标题、遮罩与表单均在弹窗内", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal-mask").exists()).toBe(true);
    expect(wrapper.find(".modal-title").text()).toBe("添加模型提供方");
    expect(wrapper.find(".modal-body .model-provider-form").exists()).toBe(true);
    expect(wrapper.find(".modal-foot .btn.primary").text()).toBe("添加");
  });

  it("编辑提供方以弹窗呈现：标题为编辑且标识只读", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".model-provider-row")[1]
      .find(".provider-row-edit")
      .trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal-title").text()).toBe("编辑模型提供方");
    expect(
      wrapper.find('input[placeholder="如 my-provider"]').attributes("disabled"),
    ).toBeDefined();
    expect(wrapper.find(".modal-foot .btn.primary").text()).toBe("保存修改");
  });

  it("提供方弹窗取消/×关闭且列表不变，重新打开表单复位", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    await wrapper.find('input[placeholder="如 my-provider"]').setValue("temp");
    // 点击遮罩不关闭（maskClose=false）
    await wrapper.find(".modal-mask").trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal-mask").exists()).toBe(true);
    // × 关闭
    await wrapper.find(".modal-close").trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal-mask").exists()).toBe(false);
    expect(wrapper.findAll(".model-provider-row").length).toBe(2);
    // 重新打开表单复位
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    expect(
      (wrapper.find('input[placeholder="如 my-provider"]')
        .element as HTMLInputElement).value,
    ).toBe("");
    // 取消关闭
    await wrapper.find(".modal-foot .btn:not(.primary)").trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal-mask").exists()).toBe(false);
  });

  it("编辑提供方后保存调用 config/batchWrite", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const rows = wrapper.findAll(".model-provider-row");
    await rows[1].find(".provider-row-edit").trigger("click");
    await flushPromises();
    expect(wrapper.find(".model-provider-form").exists()).toBe(true);
    expect(
      wrapper.find('input[placeholder="如 my-provider"]').attributes("disabled"),
    ).toBeDefined();
    await wrapper
      .find('input[placeholder="https://api.example.com/v1"]')
      .setValue("https://new.example.com/v1");
    await wrapper.find(".provider-form-submit").trigger("click");
    await flushPromises();
    await wrapper
      .findAll(".model-config-card")[0]
      .find(".model-config-save-btn")!
      .trigger("click");
    await flushPromises();
    const saveCall = mockedInvoke.mock.calls.find(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "config/batchWrite",
    );
    expect(saveCall).toBeTruthy();
    const edits = (
      saveCall![1] as {
        params: { edits: { keyPath: string; value: unknown }[] };
      }
    ).params.edits;
    expect(edits[0].keyPath).toBe("model_providers");
    expect(edits[0].value).toEqual({
      deepseek: {
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/",
        experimental_bearer_token: "sk-test",
        wire_api: "responses",
      },
      other: {
        name: "Other",
        base_url: "https://new.example.com/v1",
        env_key: "OTHER_API_KEY",
        wire_api: "chat",
      },
    });
    expect(edits[1]).toEqual({
      keyPath: "model_provider",
      value: "deepseek",
      mergeStrategy: "replace",
    });
    expect(store.toast).toContain("模型配置已保存");
  });

  it("删除激活项按钮禁用，非激活项可删除", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const rows = wrapper.findAll(".model-provider-row");
    const delBtns = rows.map(
      (r) => r.find(".provider-row-delete")!,
    );
    expect(delBtns[0].attributes("disabled")).toBeDefined();
    expect(delBtns[1].attributes("disabled")).toBeUndefined();
    expect(delBtns[1].classes()).toContain("danger");
    await delBtns[1].trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".model-provider-row").length).toBe(1);
  });

  it("不再展示固定值 UI", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    expect(wrapper.find(".model-config-fixed").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("固定值");
  });

  it("model 为空时保存被拦截并提示必填", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        return Promise.resolve({
          config: {},
          layers: [
            {
              name: { type: "user" },
              config: {
                model: "",
                model_reasoning_effort: "",
                model_provider: "deepseek",
                preferred_auth_method: "",
                forced_login_method: "",
                model_catalog_json: "",
                model_providers:
                  sampleModelConfigRead.layers[0].config.model_providers,
              },
            },
          ],
        });
      }
      if (cmd === "model_config_read")
        return Promise.resolve({
          ...sampleModelConfig,
          model: "",
          model_reasoning_effort: "",
          preferred_auth_method: "",
          forced_login_method: "",
        });
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".model-config-card")[0]
      .find(".model-config-save-btn")!
      .trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("请填写 model");
    expect(
      mockedInvoke.mock.calls.some(
        ([cmd, args]) =>
          cmd === "codex_rpc" &&
          (args as { method?: string } | undefined)?.method === "config/batchWrite",
      ),
    ).toBe(false);
  });

  it("认证方式默认不写入，选 API Key 时写入两个键", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        return Promise.resolve({
          config: {},
          layers: [
            {
              name: { type: "user" },
              config: {
                model: "deepseek-v4-flash",
                model_reasoning_effort: "high",
                model_provider: "deepseek",
                preferred_auth_method: "",
                forced_login_method: "",
                model_catalog_json: "",
                model_providers:
                  sampleModelConfigRead.layers[0].config.model_providers,
              },
            },
          ],
        });
      }
      if (cmd === "model_config_read")
        return Promise.resolve({
          ...sampleModelConfig,
          preferred_auth_method: "",
          forced_login_method: "",
        });
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    const save = () =>
      wrapper
        .findAll(".model-config-card")[0]
        .find(".model-config-save-btn")!
        .trigger("click");
    const lastAuthEdits = () => {
      const calls = mockedInvoke.mock.calls.filter(
        ([cmd, args]) =>
          cmd === "codex_rpc" &&
          (args as { method?: string } | undefined)?.method === "config/batchWrite",
      );
      const last = calls[calls.length - 1];
      return (
        last![1] as {
          params: { edits: { keyPath: string; value: string }[] };
        }
      ).params.edits;
    };
    await save();
    await flushPromises();
    let edits = lastAuthEdits();
    expect(
      edits.find((e) => e.keyPath === "preferred_auth_method")?.value,
    ).toBe("");
    expect(
      edits.find((e) => e.keyPath === "forced_login_method")?.value,
    ).toBe("");

    await wrapper.find("#model-config-ui-auth").setValue("apikey");
    await save();
    await flushPromises();
    edits = lastAuthEdits();
    expect(
      edits.find((e) => e.keyPath === "preferred_auth_method")?.value,
    ).toBe("apikey");
    expect(
      edits.find((e) => e.keyPath === "forced_login_method")?.value,
    ).toBe("api");
  });

  it("model_catalog_json 目标不存在时黄色警告但允许保存", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        return Promise.resolve({
          config: {},
          layers: [
            {
              name: { type: "user" },
              config: {
                model: "deepseek-v4-flash",
                model_reasoning_effort: "high",
                model_provider: "deepseek",
                preferred_auth_method: "",
                forced_login_method: "",
                model_catalog_json: "D:/missing/models.json",
                model_providers:
                  sampleModelConfigRead.layers[0].config.model_providers,
              },
            },
          ],
        });
      }
      if (cmd === "model_config_read")
        return Promise.resolve({
          ...sampleModelConfig,
          model_catalog_json: "D:/missing/models.json",
          preferred_auth_method: "",
          forced_login_method: "",
        });
      if (cmd === "model_catalog_target_exists")
        return Promise.resolve(false);
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".model-config-card")[0]
      .find(".model-config-save-btn")!
      .trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("model_catalog_target_exists", {
      value: "D:/missing/models.json",
    });
    expect(wrapper.text()).toContain(
      "model_catalog_json 目标文件将在读取时自动创建",
    );
    const saveCall = mockedInvoke.mock.calls.find(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "config/batchWrite",
    );
    expect(saveCall).toBeTruthy();
    const edits = (
      saveCall![1] as {
        params: { edits: { keyPath: string; value: unknown }[] };
      }
    ).params.edits;
    expect(
      edits.find((e) => e.keyPath === "model_catalog_json")?.value,
    ).toBe("D:/missing/models.json");
  });

  it("检测到全局 OPENAI_API_KEY 时表单可省略认证并显示提示", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        return Promise.resolve({
          config: {},
          layers: [
            {
              name: { type: "user" },
              config: {
                model: "",
                model_reasoning_effort: "",
                model_provider: "",
                preferred_auth_method: "",
                forced_login_method: "",
                model_catalog_json: "",
                model_providers: {},
              },
            },
          ],
        });
      }
      if (cmd === "model_config_read")
        return Promise.resolve({
          ...sampleModelConfig,
          model: "",
          model_reasoning_effort: "",
          model_provider: "",
          providers: [],
          preferred_auth_method: "",
          forced_login_method: "",
          openai_api_key_present: true,
        });
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    await wrapper
      .find('input[placeholder="如 my-provider"]')
      .setValue("env-only");
    await wrapper
      .find('input[placeholder="如 DeepSeek"]')
      .setValue("Env Only");
    await wrapper
      .find('input[placeholder="https://api.example.com/v1"]')
      .setValue("https://env.example.com/v1");
    await flushPromises();
    // 表单打开期间应看到全局 OPENAI_API_KEY 的绿色提示
    expect(wrapper.text()).toContain("已检测到全局 OPENAI_API_KEY");
    await wrapper.find(".provider-form-submit").trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".model-provider-row").length).toBe(1);
  });

  it("提供方表单缺少名称/base_url/认证时逐字段提示", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    const add = () =>
      wrapper.find(".provider-form-submit").trigger("click");
    await wrapper.find('input[placeholder="如 my-provider"]').setValue("x");
    await add();
    expect(wrapper.text()).toContain("请填写提供方名称");
    await wrapper.find('input[placeholder="如 DeepSeek"]').setValue("X");
    await add();
    expect(wrapper.text()).toContain("请填写 base_url");
    await wrapper
      .find('input[placeholder="https://api.example.com/v1"]')
      .setValue("https://x.example.com/v1");
    await add();
    expect(wrapper.text()).toContain("请填写 env_key 或 API Key");
    expect(wrapper.findAll(".model-provider-row").length).toBe(2);
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

describe("SettingsView 技能管理 / MCP 管理", () => {
  /** config/read 返回：用户层原始 [mcp_servers.*]（与真实 app-server 返回形状一致） */
  const sampleMcpReadResponse = {
    config: {},
    layers: [
      {
        name: {
          type: "user",
          file: "C:/apps/codex-ui/.codex/config.toml",
          profile: null,
        },
        version: "sha256:abc",
        config: {
          mcp_servers: {
            filesystem: {
              command: "npx",
              args: ["-y", "mcp-server-filesystem"],
              env: { K1: "v1" },
            },
            remote: {
              url: "https://example.com/mcp",
              http_headers: { Authorization: "Bearer x" },
              bearer_token_env_var: "MY_MCP_TOKEN",
            },
          },
        },
        disabledReason: null,
      },
    ],
  };
  const sampleSkillsState = {
    skills_dir: "C:/apps/codex-ui/.codex/skills",
    errors: [],
    items: [
      {
        name: "pdf",
        path: "C:/apps/codex-ui/.codex/skills/pdf/SKILL.md",
        description: "读写 PDF 文件",
        enabled: true,
      },
      {
        name: "csharp-code-rules",
        path: "C:/apps/codex-ui/.codex/skills/csharp-code-rules/SKILL.md",
        description: "C# 团队规范",
        enabled: false,
      },
    ],
  };

  beforeEach(() => {
    store.toast = "";
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "skills_read") return Promise.resolve(sampleSkillsState);
      if (cmd === "codex_rpc" && args?.method === "config/read")
        return Promise.resolve(sampleMcpReadResponse);
      if (cmd === "model_config_read")
        return Promise.resolve({
          config_path: "C:/apps/codex-ui/.codex/config.toml",
          config_exists: true,
          config_content: "",
          model_catalog_json: "",
          model_catalog_path: "",
          model_catalog_exists: false,
          model_catalog: "",
          model: "",
          model_reasoning_effort: "",
          model_provider: "",
          preferred_auth_method: "",
          forced_login_method: "",
          openai_api_key_present: false,
          providers: [],
        });
      if (cmd === "custom_instructions_read")
        return Promise.resolve({
          agents_path: "C:/apps/codex-ui/.codex/AGENTS.md",
          exists: true,
          content: "",
        });
      return Promise.resolve(undefined);
    });
    mockedSave.mockClear();
    mockedOpenPathInApp.mockReset();
    mockedOpenPathInApp.mockResolvedValue(true);
  });

  it("技能管理渲染本地技能并可打开 SKILL.md", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const nav = wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!;
    await nav.trigger("click");
    await flushPromises();
    const rows = wrapper.findAll(".skill-row");
    expect(rows.length).toBe(2);
    expect(rows[0].text()).toContain("pdf");
    expect(rows[0].text()).toContain("读写 PDF 文件");
    expect(rows[1].text()).toContain("csharp-code-rules");
    expect(rows[1].text()).toContain("C# 团队规范");
    await rows[0].find("button.skill-row-main").trigger("click");
    await flushPromises();
    expect(mockedOpenPathInApp).toHaveBeenCalledWith(
      "C:/apps/codex-ui/.codex/skills/pdf/SKILL.md",
    );
  });

  it("技能管理刷新重新拉取 skills_read", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const before = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    await wrapper
      .find(".settings-section-skills .model-config-reload-btn")
      .trigger("click");
    await flushPromises();
    const after = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    expect(after).toBe(before + 1);
  });

  it("技能为空但有加载错误时展示格式问题提示", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    mockedInvoke.mockImplementationOnce((cmd: string) => {
      if (cmd === "skills_read")
        return Promise.resolve({
          skills_dir: "C:/apps/codex-ui/.codex/skills",
          items: [],
          errors: [
            {
              message: "missing field `description`",
              path: "C:/apps/codex-ui/.codex/skills/bad/SKILL.md",
            },
          ],
        });
      return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
    });
    await wrapper
      .find(".settings-section-skills .model-config-reload-btn")
      .trigger("click");
    await flushPromises();
    const empty = wrapper.find(".settings-section-skills .plugin-empty");
    expect(empty.text()).toContain("1 个技能因格式问题未加载");
    expect(empty.text()).toContain("missing field `description`");
  });

  it("禁用技能行显示状态与启用按钮，点击后写配置并强制刷新", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    const rows = wrapper.findAll(".skill-row");
    expect(rows[1].text()).toContain("已禁用");
    await rows[1].find(".skill-actions .btn").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "skills/config/write",
      params: { name: "csharp-code-rules", enabled: true },
    });
    expect(mockedInvoke).toHaveBeenCalledWith("skills_read", {
      forceReload: true,
    });
    expect(store.toast).toContain("已启用 csharp-code-rules");
  });

  it("启用状态的技能点击后写禁用配置", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    const rows = wrapper.findAll(".skill-row");
    expect(rows[0].text()).toContain("已启用");
    await rows[0].find(".skill-actions .btn").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "skills/config/write",
      params: { name: "pdf", enabled: false },
    });
    expect(store.toast).toContain("已禁用 pdf");
  });

  it("启用/禁用失败时 toast 错误且不强制刷新", async () => {
    mockedInvoke.mockImplementation(async (cmd, args) => {
      if (cmd === "skills_read") return Promise.resolve(sampleSkillsState);
      const params = args as Record<string, unknown> | undefined;
      if (cmd === "codex_rpc" && params?.method === "skills/config/write") {
        throw new Error("配置写入失败");
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    const rows = wrapper.findAll(".skill-row");
    await rows[1].find(".skill-actions .btn").trigger("click");
    await flushPromises();
    expect(store.toast).toContain("配置写入失败");
    const forcedReloads = mockedInvoke.mock.calls.filter(
      ([name, arg]) =>
        name === "skills_read" &&
        (arg as Record<string, unknown> | undefined)?.forceReload === true,
    );
    expect(forcedReloads.length).toBe(0);
  });

  it("MCP 管理渲染服务器名称与传输类型", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".mcp-server-row").length).toBe(2);
    expect(wrapper.text()).toContain("filesystem");
    expect(wrapper.text()).toContain("remote");
    expect(
      wrapper.findAll(".mcp-server-type").map((t) => t.text()),
    ).toEqual(["stdio", "http"]);
    expect(wrapper.find(".mcp-row-delete").classes()).toContain("danger");
    expect(
      wrapper.find(".settings-section-mcp .mcp-config-add-btn").exists(),
    ).toBe(true);
    // 即时保存模式下 MCP 卡不再有「保存」按钮
    expect(
      wrapper
        .find(".settings-section-mcp .model-config-card")
        .findAll("button")
        .some((b) => b.text().includes("保存")),
    ).toBe(false);
  });

  it("MCP stdio 添加：args 空格分隔后保存", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();

    await wrapper.find(".mcp-config-add-btn").trigger("click");
    await flushPromises();
    // 新增默认勾选 deferred（直接内联），direct/code_mode 不勾选
    expect(
      (
        wrapper.find('.mcp-server-form .mcp-omit-option input[value="deferred"]')
          .element as HTMLInputElement
      ).checked,
    ).toBe(true);
    expect(
      (
        wrapper.find('.mcp-server-form .mcp-omit-option input[value="direct"]')
          .element as HTMLInputElement
      ).checked,
    ).toBe(false);
    await wrapper
      .find('.mcp-server-form input[placeholder="如 filesystem"]')
      .setValue("memory");
    await wrapper
      .find('.mcp-server-form input[placeholder="如 npx"]')
      .setValue("npx");
    await wrapper
      .find(".mcp-server-form .mcp-args-input")
      .setValue("-y mcp-server-memory");
    await wrapper.find(".mcp-kv-add-btn").trigger("click");
    await flushPromises();
    const envInputs = wrapper.findAll(".mcp-env-row input");
    await envInputs[0].setValue("API_KEY");
    await envInputs[1].setValue("sk-123");
    await wrapper.find(".mcp-form-submit").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "config/batchWrite",
      params: {
        edits: [
          {
            keyPath: "mcp_servers",
            value: {
              filesystem: {
                command: "npx",
                args: ["-y", "mcp-server-filesystem"],
                env: { K1: "v1" },
              },
              remote: {
                url: "https://example.com/mcp",
                http_headers: { Authorization: "Bearer x" },
                bearer_token_env_var: "MY_MCP_TOKEN",
              },
              memory: {
                command: "npx",
                args: ["-y", "mcp-server-memory"],
                env: { API_KEY: "sk-123" },
                omit_tools_from: ["deferred"],
              },
            },
            mergeStrategy: "replace",
          },
        ],
        reloadUserConfig: true,
      },
    });
    expect(store.toast).toContain("MCP 服务器已保存");
  });

  it("MCP http 添加：url/请求头/bearer 令牌保存", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    await wrapper.find(".mcp-config-add-btn").trigger("click");
    await flushPromises();
    await wrapper.find("#mcp-form-transport").setValue("http");
    await wrapper
      .find('.mcp-server-form input[placeholder="如 filesystem"]')
      .setValue("docs");
    await wrapper
      .find('.mcp-server-form input[placeholder="如 https://example.com/mcp"]')
      .setValue("https://developers.openai.com/mcp");
    await wrapper
      .find('.mcp-server-form input[placeholder="如 MY_MCP_TOKEN"]')
      .setValue("DOCS_TOKEN");
    await wrapper.find(".mcp-kv-add-btn").trigger("click");
    await flushPromises();
    const headerInputs = wrapper.findAll(".mcp-env-row input");
    await headerInputs[0].setValue("Authorization");
    await headerInputs[1].setValue("Bearer sk-docs");
    await wrapper.find(".mcp-form-submit").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "config/batchWrite",
      params: {
        edits: [
          {
            keyPath: "mcp_servers",
            value: {
              filesystem: {
                command: "npx",
                args: ["-y", "mcp-server-filesystem"],
                env: { K1: "v1" },
              },
              remote: {
                url: "https://example.com/mcp",
                http_headers: { Authorization: "Bearer x" },
                bearer_token_env_var: "MY_MCP_TOKEN",
              },
              docs: {
                url: "https://developers.openai.com/mcp",
                http_headers: { Authorization: "Bearer sk-docs" },
                bearer_token_env_var: "DOCS_TOKEN",
                omit_tools_from: ["deferred"],
              },
            },
            mergeStrategy: "replace",
          },
        ],
        reloadUserConfig: true,
      },
    });
  });

  it("MCP 空 command 被拦截", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    await wrapper.find(".mcp-config-add-btn").trigger("click");
    await flushPromises();
    await wrapper
      .find('.mcp-server-form input[placeholder="如 filesystem"]')
      .setValue("x");
    await wrapper.find(".mcp-form-submit").trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("请填写 command");
    expect(wrapper.findAll(".mcp-server-row").length).toBe(2);
  });

  it("MCP http 空 url 被拦截", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    await wrapper.find(".mcp-config-add-btn").trigger("click");
    await flushPromises();
    await wrapper.find("#mcp-form-transport").setValue("http");
    await wrapper
      .find('.mcp-server-form input[placeholder="如 filesystem"]')
      .setValue("x");
    await wrapper.find(".mcp-form-submit").trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("请填写 url");
    expect(wrapper.findAll(".mcp-server-row").length).toBe(2);
  });

  it("MCP 删除弹确认后立即保存", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    await wrapper
      .findAll(".mcp-server-row")[0]
      .find(".mcp-row-delete")
      .trigger("click");
    await flushPromises();
    settleConfirm(true);
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "config/batchWrite",
      params: {
        edits: [
          {
            keyPath: "mcp_servers",
            value: {
              remote: {
                url: "https://example.com/mcp",
                http_headers: { Authorization: "Bearer x" },
                bearer_token_env_var: "MY_MCP_TOKEN",
              },
            },
            mergeStrategy: "replace",
          },
        ],
        reloadUserConfig: true,
      },
    });
    expect(store.toast).toContain("已删除 MCP 服务器");
  });

  it("MCP 删除取消时不落盘", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    await wrapper
      .findAll(".mcp-server-row")[0]
      .find(".mcp-row-delete")
      .trigger("click");
    settleConfirm(false);
    await flushPromises();
    expect(wrapper.findAll(".mcp-server-row").length).toBe(2);
    expect(
      mockedInvoke.mock.calls.filter(
        ([name, args]) =>
          name === "codex_rpc" &&
          (args as { method?: string } | undefined)?.method ===
            "config/batchWrite",
      ).length,
    ).toBe(0);
  });

  it("新增 MCP 服务器以弹窗呈现，取消/×关闭且列表不变", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    await wrapper.find(".mcp-config-add-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal-mask").exists()).toBe(true);
    expect(wrapper.find(".modal-title").text()).toBe("添加 MCP 服务器");
    expect(wrapper.find(".modal-body .mcp-server-form").exists()).toBe(true);
    expect(wrapper.find(".modal-foot .btn.primary").text()).toBe("添加");
    await wrapper.find(".modal-close").trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal-mask").exists()).toBe(false);
    expect(wrapper.findAll(".mcp-server-row").length).toBe(2);
    // 重新打开后传输方式回到 stdio、名称为空
    await wrapper.find(".mcp-config-add-btn").trigger("click");
    await flushPromises();
    expect(
      (wrapper.find("#mcp-form-transport").element as HTMLSelectElement).value,
    ).toBe("stdio");
    expect(
      (wrapper.find('.mcp-server-form input[placeholder="如 filesystem"]')
        .element as HTMLInputElement).value,
    ).toBe("");
    await wrapper.find(".modal-foot .btn:not(.primary)").trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal-mask").exists()).toBe(false);
  });

  it("编辑 MCP 服务器弹窗标题为编辑且名称只读", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    await wrapper
      .findAll(".mcp-server-row")[0]
      .find(".mcp-row-edit")
      .trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal-title").text()).toBe("编辑 MCP 服务器");
    expect(
      wrapper
        .find('.mcp-server-form input[placeholder="如 filesystem"]')
        .attributes("disabled"),
    ).toBeDefined();
    expect(wrapper.find(".modal-foot .btn.primary").text()).toBe("保存修改");
  });

  it("新增 MCP 取消全部勾选保存后不含 omit_tools_from 字段", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    await wrapper.find(".mcp-config-add-btn").trigger("click");
    await flushPromises();
    // 取消默认勾选的 deferred（回到 codex 默认，即延迟暴露）
    await wrapper
      .find('.mcp-server-form .mcp-omit-option input[value="deferred"]')
      .setValue(false);
    await wrapper
      .find('.mcp-server-form input[placeholder="如 filesystem"]')
      .setValue("xapi");
    await wrapper
      .find('.mcp-server-form input[placeholder="如 npx"]')
      .setValue("npx");
    await wrapper.find(".mcp-form-submit").trigger("click");
    await flushPromises();
    const batchWriteCall = mockedInvoke.mock.calls.find(
      ([name, args]) =>
        name === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "config/batchWrite",
    ) as [string, { params: { edits: { value: Record<string, Record<string, unknown>> }[] } }];
    const value = batchWriteCall[1].params.edits[0].value;
    expect(value.xapi.omit_tools_from).toBeUndefined();
    expect(value.xapi).toEqual({ command: "npx" });
  });

  it("编辑 MCP 表单按当前 omit_tools_from 勾选；未配置的不勾选", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read")
        return Promise.resolve({
          config: {},
          layers: [
            {
              name: { type: "user", file: "C:/x/.codex/config.toml", profile: null },
              version: "x",
              config: {
                mcp_servers: {
                  cfg: { command: "npx", omit_tools_from: ["code_mode"] },
                  plain: { command: "npx" },
                },
              },
              disabledReason: null,
            },
          ],
        });
      if (cmd === "skills_read") return Promise.resolve(sampleSkillsState);
      if (cmd === "model_config_read")
        return Promise.resolve({
          config_path: "C:/apps/codex-ui/.codex/config.toml",
          config_exists: true,
          config_content: "",
          model_catalog_json: "",
          model_catalog_path: "",
          model_catalog_exists: false,
          model_catalog: "",
          model: "",
          model_reasoning_effort: "",
          model_provider: "",
          preferred_auth_method: "",
          forced_login_method: "",
          openai_api_key_present: false,
          providers: [],
        });
      if (cmd === "custom_instructions_read")
        return Promise.resolve({
          agents_path: "C:/apps/codex-ui/.codex/AGENTS.md",
          exists: true,
          content: "",
        });
      return Promise.resolve(undefined);
    });

    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    // 编辑 cfg：code_mode 勾选，deferred/direct 不勾选
    await wrapper
      .findAll(".mcp-server-row")[0]
      .find(".mcp-row-edit")
      .trigger("click");
    await flushPromises();
    expect(
      (
        wrapper.find('.mcp-server-form .mcp-omit-option input[value="code_mode"]')
          .element as HTMLInputElement
      ).checked,
    ).toBe(true);
    expect(
      (
        wrapper.find('.mcp-server-form .mcp-omit-option input[value="deferred"]')
          .element as HTMLInputElement
      ).checked,
    ).toBe(false);
    await wrapper.find(".modal-foot .btn:not(.primary)").trigger("click");
    await flushPromises();
    // 编辑 plain：未配置 => 全不勾选
    await wrapper
      .findAll(".mcp-server-row")[1]
      .find(".mcp-row-edit")
      .trigger("click");
    await flushPromises();
    expect(
      (
        wrapper.find('.mcp-server-form .mcp-omit-option input[value="deferred"]')
          .element as HTMLInputElement
      ).checked,
    ).toBe(false);
    expect(
      (
        wrapper.find('.mcp-server-form .mcp-omit-option input[value="code_mode"]')
          .element as HTMLInputElement
      ).checked,
    ).toBe(false);
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

  it("个性化 / 通用设置 / 模型配置 / 技能管理 / MCP管理 / 插件管理六个分区均渲染", () => {
    wrapper = mount(SettingsView);
    const titles = wrapper.findAll(".settings-section-title").map((s) => s.text());
    expect(titles).toContain("个性化");
    expect(titles).toContain("模型配置");
    expect(titles).toContain("通用设置");
    expect(titles).toContain("技能管理");
    expect(titles).toContain("MCP 管理");
    expect(titles).toContain("插件管理");
  });

  it("左侧导航渲染六个分类，默认选中第一个", () => {
    wrapper = mount(SettingsView);
    const items = wrapper.findAll(".settings-nav-item");
    expect(items.map((i) => i.text().trim())).toEqual([
      "个性化",
      "通用设置",
      "模型配置",
      "技能管理",
      "MCP管理",
      "插件管理",
    ]);
    expect(items[0].classes()).toContain("active");
    expect(items[1].classes()).not.toContain("active");
    expect(items[2].classes()).not.toContain("active");
    expect(items[3].classes()).not.toContain("active");
    expect(items[4].classes()).not.toContain("active");
    expect(items[5].classes()).not.toContain("active");
    const personal = wrapper
      .find(".settings-section-personalization")
      .element as HTMLElement;
    const general = wrapper.find(".settings-section-general").element as HTMLElement;
    const skills = wrapper.find(".settings-section-skills").element as HTMLElement;
    const mcp = wrapper.find(".settings-section-mcp").element as HTMLElement;
    const plugins = wrapper.find(".settings-section-plugins").element as HTMLElement;
    expect(personal.style.display).not.toBe("none");
    expect(general.style.display).toBe("none");
    expect(skills.style.display).toBe("none");
    expect(mcp.style.display).toBe("none");
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
      expect.arrayContaining([
        "只读访问",
        "请求批准",
        "帮我批准",
        "完全访问权限",
      ]),
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

  it("按市场分组渲染插件与状态标签，头部显示插件数量", async () => {
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
    // 默认折叠：插件列表隐藏
    expect(wrapper.find(".plugin-list").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Browser");
    expect(wrapper.text()).not.toContain("PDF");
    expect(wrapper.text()).not.toContain("Gmail");
    // 头部（市场名/插件数量）始终可见
    expect(mps[0].find(".plugin-marketplace-count").text()).toBe("2");
    expect(mps[1].find(".plugin-marketplace-count").text()).toBe("1");
    // 展开后可见插件名与状态
    await mps[0].find(".plugin-marketplace-head").trigger("click");
    await mps[1].find(".plugin-marketplace-head").trigger("click");
    expect(wrapper.text()).toContain("Browser");
    expect(wrapper.text()).toContain("PDF");
    expect(wrapper.text()).toContain("Gmail");
    expect(wrapper.text()).toContain("已启用");
    expect(wrapper.text()).toContain("未安装");
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
    // 默认折叠
    expect(wrapper.find(".plugin-list").exists()).toBe(false);
    const head = wrapper.find(".plugin-marketplace-head");
    expect(head.classes()).toContain("collapsed");
    // 点击展开
    await head.trigger("click");
    expect(wrapper.find(".plugin-list").exists()).toBe(true);
    expect(head.classes()).not.toContain("collapsed");
    // 再点击折叠
    await head.trigger("click");
    expect(wrapper.find(".plugin-list").exists()).toBe(false);
    expect(head.classes()).toContain("collapsed");
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
    await head.trigger("click"); // 先展开，验证移除不把它折叠
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

  it("带本地路径的市场插件安装传 marketplacePath", async () => {
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
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    await wrapper.find(".plugin-install-btn").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "plugin/install",
      params: { marketplacePath: "C:/x/bundled", pluginName: "pdf" },
    });
    expect(store.toast).toContain("已安装 PDF");
  });

  it("带本地路径的市场 verbatim 路径剥离 \\?\\ 后传 marketplacePath", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "plugin/list") {
        return {
          marketplaces: [
            {
              name: "openai-primary-runtime",
              path: "\\\\?\\C:\\Users\\laojiu\\.cache\\codex-runtimes\\codex-primary-runtime\\plugins\\openai-primary-runtime",
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
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    await wrapper.find(".plugin-install-btn").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "plugin/install",
      params: {
        marketplacePath:
          "C:\\Users\\laojiu\\.cache\\codex-runtimes\\codex-primary-runtime\\plugins\\openai-primary-runtime",
        pluginName: "pdf",
      },
    });
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
    await wrapper.find(".plugin-marketplace-head").trigger("click");
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
    await wrapper.find(".plugin-marketplace-head").trigger("click");
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
    await wrapper.find(".plugin-marketplace-head").trigger("click");
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
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    expect(wrapper.text()).toContain("管理员已禁用");
    expect(wrapper.find(".plugin-install-btn").attributes("disabled")).toBeDefined();
  });

  it("插件带远程图标时渲染 img 且 src 正确", async () => {
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
              interface: {
                displayName: "Browser",
                composerIconUrl: "https://cdn.example/browser.png",
              },
            },
          ],
        },
      ],
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    const img = wrapper.find(".plugin-row-icon img");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toBe("https://cdn.example/browser.png");
    expect(wrapper.find(".plugin-icon-fallback").exists()).toBe(false);
  });

  it("插件本地图标路径经 assetUrl 转 asset://", async () => {
    mockedInvoke.mockResolvedValue({
      marketplaces: [
        {
          name: "openai-bundled",
          path: "C:/x/bundled",
          plugins: [
            {
              id: "pdf",
              name: "pdf",
              installed: false,
              enabled: false,
              interface: {
                displayName: "PDF",
                composerIcon: "C:/x/icon.png",
              },
            },
          ],
        },
      ],
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    const img = wrapper.find(".plugin-row-icon img");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toBe("asset://mock/C:/x/icon.png");
  });

  it("插件无任何图标但有品牌色时渲染首字母回退", async () => {
    mockedInvoke.mockResolvedValue({
      marketplaces: [
        {
          name: "openai-bundled",
          path: "C:/x/bundled",
          plugins: [
            {
              id: "chrome",
              name: "chrome",
              installed: false,
              enabled: false,
              interface: {
                displayName: "Chrome",
                brandColor: "#4678EB",
              },
            },
          ],
        },
      ],
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    const fallback = wrapper.find(".plugin-icon-fallback");
    expect(wrapper.find(".plugin-row-icon img").exists()).toBe(false);
    expect(fallback.exists()).toBe(true);
    expect(fallback.text()).toBe("C");
    expect(fallback.attributes("style")).toContain("#4678EB");
  });

  it("图标加载失败后切换到品牌色回退", async () => {
    mockedInvoke.mockResolvedValue({
      marketplaces: [
        {
          name: "openai-bundled",
          path: "C:/x/bundled",
          plugins: [
            {
              id: "broken",
              name: "broken",
              installed: false,
              enabled: false,
              interface: {
                displayName: "Broken",
                composerIconUrl: "https://cdn.example/broken.png",
                brandColor: "#ff0000",
              },
            },
          ],
        },
      ],
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    const img = wrapper.find(".plugin-row-icon img");
    expect(img.exists()).toBe(true);
    await img.trigger("error");
    expect(wrapper.find(".plugin-row-icon img").exists()).toBe(false);
    const fallback = wrapper.find(".plugin-icon-fallback");
    expect(fallback.exists()).toBe(true);
    expect(fallback.text()).toBe("B");
    expect(fallback.attributes("style")).toContain("#ff0000");
  });
});

describe("SettingsView 按钮图标", () => {
  let wrapper: ReturnType<typeof mount> | undefined;

  const sampleModelConfig = {
    config_path: "C:/apps/codex-ui/.codex/config.toml",
    config_exists: true,
    config_content: 'model = "deepseek-v4-flash"\n',
    model_catalog_path: "C:/apps/codex-ui/.codex/models.json",
    model_catalog_exists: true,
    model_catalog: '{\n  "models": []\n}',
    model: "deepseek-v4-flash",
    model_reasoning_effort: "",
    model_provider: "deepseek",
    providers: [
      {
        key: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/",
        experimental_bearer_token: "sk-test",
        wire_api: "responses",
      },
    ],
  };
  const sampleAgentsState = {
    agents_path: "C:/apps/codex-ui/.codex/AGENTS.md",
    exists: true,
    content: "# AGENTS.md\n",
  };

  beforeEach(() => {
    __resetTabsForTest();
    __resetSessionTabsForTest();
    store.toast = "";
    store.confirm = null;
    store.settings.codex_path = "C:/tools/codex.exe";
    mockedInvoke.mockReset();
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
      if (cmd === "model_config_read") return Promise.resolve(sampleModelConfig);
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        return Promise.resolve({
          config: {},
          layers: [
            {
              name: { type: "user" },
              config: {
                model: "deepseek-v4-flash",
                model_reasoning_effort: "",
                model_provider: "deepseek",
                preferred_auth_method: "",
                forced_login_method: "",
                model_catalog_json: "",
                model_providers: {
                  deepseek: {
                    name: "DeepSeek",
                    base_url: "https://api.deepseek.com/",
                    experimental_bearer_token: "sk-test",
                    wire_api: "responses",
                  },
                },
              },
            },
          ],
        });
      }
      return Promise.resolve({});
    });
    mockedSave.mockClear();
    mockedSave.mockResolvedValue(undefined);
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
  });

  it("导航与主要操作按钮均渲染图标且文案不以省略号结尾", async () => {
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    // 打开提供方弹窗，覆盖弹窗底部按钮（取消/添加）
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    const selectors = [
      ".settings-nav-item",
      ".model-config-path-link",
      ".model-config-actions button.primary",
      ".model-config-reload-btn",
      ".model-provider-actions .btn",
      "button.codex-pick-btn",
      "button.codex-clear-btn",
      "button.memory-reset-btn",
      ".plugin-refresh-btn",
      ".plugin-market-add .btn",
      ".plugin-market-remove",
      ".plugin-install-btn",
      ".plugin-uninstall-btn",
    ];
    for (const sel of selectors) {
      const buttons = wrapper.findAll(sel);
      expect(buttons.length, `${sel} 未找到按钮`).toBeGreaterThan(0);
      for (const btn of buttons) {
        expect(btn.find("svg").exists(), `${sel} 缺少图标`).toBe(true);
        expect(
          btn.text().trim().endsWith("…"),
          `${sel} 文案不应以省略号结尾`,
        ).toBe(false);
      }
    }
    // 弹窗底部为文字按钮（取消/添加），无需图标但文案完整
    const footButtons = wrapper.findAll(".modal-foot .btn");
    expect(footButtons.length).toBe(2);
    for (const btn of footButtons) {
      expect(btn.text().trim().length).toBeGreaterThan(0);
      expect(btn.text().trim().endsWith("…")).toBe(false);
    }
  });
});
