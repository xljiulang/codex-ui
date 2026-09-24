import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";
import { config, flushPromises, mount } from "@vue/test-utils";
import { nextTick } from "vue";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn(),
}));
vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod =
    await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, saveSettings: vi.fn() };
});
vi.mock("../../composables/useSessionFs", async (importOriginal) => {
  const mod =
    await importOriginal<typeof import("../../composables/useSessionFs")>();
  return { ...mod, openPathInApp: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
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
const mockedGetVersion = vi.mocked(getVersion);
const mockedSave = vi.mocked(saveSettings);
const mockedOpenPathInApp = vi.mocked(openPathInApp);

/** 展开 AppSelect（触发按钮由 id 定位，弹层 Teleport 到 body）并点击指定文案的选项 */
async function pickAppSelect(
  host: ReturnType<typeof mount>,
  id: string,
  label: string,
) {
  await host.find(`button#${CSS.escape(id)}`).trigger("click");
  await nextTick();
  const target = Array.from(
    document.body.querySelectorAll<HTMLElement>(".app-select-option"),
  ).find((o) => o.textContent?.trim() === label);
  expect(target).toBeTruthy();
  target!.click();
  await nextTick();
}

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
    (mockedInvoke as ReturnType<typeof vi.fn>).mockImplementation(
      async () => undefined,
    );
  });

  it("渲染只读路径与选择按钮，路径行不再渲染文本输入框", () => {
    const wrapper = mount(SettingsView);
    const row = wrapper.find(".codex-path-row");
    expect(row.exists()).toBe(true);
    expect(row.find(".codex-path-value").text()).toContain(
      "C:/tools/codex.exe",
    );
    expect(row.find("button.codex-pick-btn").attributes("data-tip")).toBe(
      "选择文件",
    );
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
    config_content:
      'model = "deepseek-v4-flash"\nmodel_reasoning_effort = "high"\n',
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
        wire_api: "responses",
      },
    ],
  };
  /** config/read 返回：用户层原始 [model_providers.*] 与顶层标量（与真实 app-server 形状一致） */
  const sampleModelConfigRead = {
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
              wire_api: "responses",
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
      if (cmd === "model_config_read")
        return Promise.resolve(sampleModelConfig);
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    mockedSave.mockClear();
    mockedOpenPathInApp.mockReset();
    mockedOpenPathInApp.mockResolvedValue(true);
  });

  it("导航顺序：个性化 → … → 定时任务 → 兼容代理 → 关于", () => {
    const wrapper = mount(SettingsView);
    const labels = wrapper
      .findAll(".settings-nav-item")
      .map((i) => i.text().trim());
    expect(labels.indexOf("个性化")).toBe(0);
    expect(labels.indexOf("基础设置")).toBe(1);
    expect(labels.indexOf("全局指令")).toBe(2);
    // 模型快照独立成 Tab，且排在剩余的「模型配置」之前
    expect(labels.indexOf("模型快照")).toBe(3);
    expect(labels.indexOf("模型配置")).toBe(4);
    expect(labels.indexOf("动态工具")).toBe(5);
    expect(labels.indexOf("技能管理")).toBe(6);
    expect(labels.indexOf("MCP管理")).toBe(7);
    expect(labels.indexOf("插件管理")).toBe(8);
    expect(labels.indexOf("定时任务")).toBe(9);
    expect(labels.indexOf("兼容代理")).toBe(10);
    expect(labels.indexOf("关于")).toBe(11);
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
    expect(wrapper.text()).toContain("模型配置");
    expect(wrapper.text()).toContain("model_catalog_json");
    expect(wrapper.text()).toContain("AGENTS");
    // 模型快照已迁出：模型配置 Tab 只剩 1 张卡，快照 Tab 自带 1 张卡
    expect(
      wrapper.findAll(".settings-section-model-config .model-config-card")
        .length,
    ).toBe(1);
    expect(
      wrapper.findAll(
        ".settings-section-global-instructions .model-config-card",
      ).length,
    ).toBe(1);
    expect(wrapper.find(".model-config-missing").exists()).toBe(false);
  });

  it("模型快照 Tab 独立渲染，还原后模型配置自动重读", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        return Promise.resolve(sampleModelConfigRead);
      }
      if (cmd === "model_config_read")
        return Promise.resolve(sampleModelConfig);
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      if (cmd === "model_snapshots_list") return Promise.resolve(["dev"]);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    const readCount = () =>
      mockedInvoke.mock.calls.filter(([name]) => name === "model_config_read")
        .length;
    expect(readCount()).toBe(1);

    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("模型快照"))!
      .trigger("click");
    await flushPromises();

    // 快照卡片归属「模型快照」Tab，卡片头小标题为「已保存快照」
    expect(
      wrapper.findAll(".settings-section-model-snapshot .model-config-card")
        .length,
    ).toBe(1);
    expect(
      wrapper
        .find(".settings-section-model-snapshot .model-config-card-head h3")
        .text(),
    ).toBe("已保存快照");
    // 模型配置 Tab 内不再有快照行
    expect(
      wrapper
        .find(".settings-section-model-config .model-snapshot-row")
        .exists(),
    ).toBe(false);

    await wrapper.find('button[aria-label="还原模型快照dev"]').trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("model_snapshots_apply", {
      name: "dev",
    });
    expect(readCount()).toBe(2);
    wrapper.unmount();
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

  it("文件缺失时可编辑，模型目录与 AGENTS 标题均为文件链接", async () => {
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
        return Promise.resolve({
          ...sampleAgentsState,
          exists: true,
          content: "",
        });
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    const textareas = wrapper.findAll("textarea.model-config-textarea");
    // 模型目录元数据与 AGENTS 均可编辑
    expect(textareas[0].attributes("disabled")).toBeUndefined();
    expect(
      wrapper
        .find("textarea.custom-instructions-textarea")
        .attributes("disabled"),
    ).toBeUndefined();
    expect(wrapper.find(".model-config-missing").exists()).toBe(false);
    // 模型目录标题即文件链接（路径行已合并进标题），AGENTS 标题保留链接
    const catalogLinks = wrapper.findAll(
      ".settings-section-model-config .model-catalog-head .model-config-title-link",
    );
    expect(catalogLinks.length).toBe(1);
    expect(catalogLinks[0].attributes("disabled")).toBeUndefined();
    expect(
      wrapper.findAll(
        ".settings-section-global-instructions .model-config-title-link",
      ).length,
    ).toBe(1);
  });

  it("模型配置卡片保留唯一保存按钮且文本为「保存」", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const saveButtons = wrapper.findAll(
      ".settings-section-model-config .model-config-card .model-config-actions .model-config-save-btn",
    );
    expect(saveButtons.length).toBe(1);
    expect(saveButtons[0].text().trim()).toBe("保存");
  });

  it("保存模型配置同时调用 model_catalog_save 与 config/batchWrite", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .find(
        ".settings-section-model-config .model-config-card .model-config-actions .model-config-save-btn",
      )
      .trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("model_catalog_save", {
      content: '{\n  "models": []\n}',
    });
    const batchWrite = mockedInvoke.mock.calls.find(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method ===
          "config/batchWrite",
    );
    expect(batchWrite).toBeTruthy();
    expect(store.toast).toContain("模型配置已保存");
    expect(store.toast).toContain("重启 codex-ui 后生效");
  });

  it("点 AGENTS 卡「保存」调用 custom_instructions_save", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const cards = wrapper.findAll(
      ".settings-section-global-instructions .model-config-card",
    );
    await cards[0]
      .find(".model-config-actions .model-config-save-btn")
      .trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("custom_instructions_save", {
      content: "# AGENTS.md\n\nWindows 环境。\n",
    });
    expect(store.toast).toContain("AGENTS 已保存");
  });

  it("模型目录标题即文件链接，点击在应用内打开", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const links = wrapper.findAll(
      ".settings-section-model-config .model-catalog-head .model-config-title-link",
    );
    expect(links.length).toBe(1);
    expect(links[0].text()).toBe("model_catalog_json（模型目录）");
    expect(links[0].attributes("aria-label")).toBe(
      "在编辑器中打开 C:/apps/codex-ui/.codex/models.json",
    );
    expect(links[0].attributes("disabled")).toBeUndefined();
    // 独立路径行已删除，路径只出现在标题链接的 tooltip 中
    expect(
      wrapper.findAll(".settings-section-model-config .model-config-path-link")
        .length,
    ).toBe(0);
    expect(
      wrapper.find(".model-catalog-title-wrap").attributes("data-tip"),
    ).toBe("在编辑器中打开 C:/apps/codex-ui/.codex/models.json");
    await links[0].trigger("click");
    await flushPromises();
    expect(mockedOpenPathInApp).toHaveBeenCalledWith(
      "C:/apps/codex-ui/.codex/models.json",
    );
  });

  it("模型配置文件不存在时标题链接禁用并给出路径提示", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        return Promise.resolve(sampleModelConfigRead);
      }
      if (cmd === "model_config_read")
        return Promise.resolve({
          ...sampleModelConfig,
          model_catalog_exists: false,
          model_catalog: "",
        });
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    const link = wrapper.find(
      ".settings-section-model-config .model-catalog-head .model-config-title-link",
    );
    expect(link.exists()).toBe(true);
    expect(link.attributes("disabled")).toBeDefined();
    expect(wrapper.find(".model-catalog-head").text()).toContain(
      "model_catalog_json（模型目录）",
    );
    // 独立路径行已删除：路径与「保存时将新建」提示只在标题链接的 tooltip 中给出
    expect(
      wrapper.find(".model-catalog-block .model-config-path").exists(),
    ).toBe(false);
    expect(
      wrapper.findAll(".settings-section-model-config .model-config-path-link")
        .length,
    ).toBe(0);
    const tip = wrapper
      .find(".model-catalog-title-wrap")
      .attributes("data-tip") as string;
    expect(tip).toContain("C:/apps/codex-ui/.codex/models.json");
    expect(tip).toContain("文件不存在，保存时将新建");
  });

  it("点击 AGENTS 标题链接在应用内打开", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const links = wrapper.findAll(
      ".settings-section-global-instructions .model-config-title-link",
    );
    await links[0].trigger("click");
    await flushPromises();
    expect(mockedOpenPathInApp).toHaveBeenCalledWith(
      "C:/apps/codex-ui/.codex/AGENTS.md",
    );
  });

  it("应用内打开失败时回退 reveal_path", async () => {
    mockedOpenPathInApp.mockResolvedValue(false);
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .find(".settings-section-global-instructions .model-config-title-link")
      .trigger("click");
    await flushPromises();
    expect(mockedOpenPathInApp).toHaveBeenCalledWith(
      "C:/apps/codex-ui/.codex/AGENTS.md",
    );
    expect(mockedInvoke).toHaveBeenCalledWith("reveal_path", {
      path: "C:/apps/codex-ui/.codex/AGENTS.md",
    });
  });

  it("渲染 DeepSeek / GLM 接入文档链接", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const links = wrapper.findAll(
      ".settings-section-model-config .model-config-docs-link",
    );
    expect(links.length).toBe(2);
    expect(links[0].text()).toBe("DeepSeek");
    expect(links[1].text()).toBe("GLM");
    expect(
      wrapper.findAll(
        ".settings-section-model-config .model-config-head-actions .model-config-docs-link",
      ).length,
    ).toBe(2);
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

  it("点击 GLM 接入文档链接用默认浏览器打开文档", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const links = wrapper.findAll(".model-config-docs-link");
    await links[1].trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("open_url", {
      url: "https://docs.bigmodel.cn/cn/coding-plan/tool/codex",
    });
  });

  it("技能管理头部渲染 Skills Catalog 链接、位于加号之前且点击打开浏览器", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    const head = wrapper.find(
      ".settings-section-skills .model-config-head-actions",
    );
    const link = head.find(".skill-catalog-link");
    expect(link.exists()).toBe(true);
    expect(link.text()).toBe("Skills Catalog for Codex");
    const buttons = head.findAll("button");
    const linkIndex = buttons.findIndex((b) =>
      b.classes().includes("skill-catalog-link"),
    );
    const addIndex = buttons.findIndex((b) =>
      b.classes().includes("skill-add-btn"),
    );
    expect(linkIndex).toBeGreaterThanOrEqual(0);
    expect(addIndex).toBeGreaterThanOrEqual(0);
    expect(linkIndex).toBeLessThan(addIndex);
    await link.trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("open_url", {
      url: "https://github.com/openai/skills",
    });
  });

  it("重读模型配置会重新从磁盘读取（含目录内容）", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .find("textarea.model-config-textarea")
      .setValue('{\n  "models": [],\n  "edited": true\n}');
    const readCallsBefore = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "config/read",
    ).length;
    await wrapper
      .find(
        ".settings-section-model-config .model-config-card .model-config-reload-btn",
      )
      .trigger("click");
    await flushPromises();
    expect(
      mockedInvoke.mock.calls.filter(
        ([cmd, args]) =>
          cmd === "codex_rpc" &&
          (args as { method?: string } | undefined)?.method === "config/read",
      ).length,
    ).toBe(readCallsBefore + 1);
    // 重读会从磁盘回填目录内容（默认 sampleModelConfig.model_catalog）
    expect(
      (
        wrapper.find("textarea.model-config-textarea")
          .element as HTMLTextAreaElement
      ).value,
    ).toBe('{\n  "models": []\n}');
  });

  it("模型配置重读与 AGENTS 刷新重新从磁盘读取", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    expect(
      wrapper
        .find(
          ".settings-section-model-config .model-config-card .model-config-reload-btn",
        )
        .attributes("aria-label"),
    ).toBe("刷新");
    const agentsCard = wrapper.find(
      ".settings-section-global-instructions .model-config-card",
    );
    expect(
      agentsCard.find(".model-config-reload-btn").attributes("aria-label"),
    ).toBe("刷新");
    const readCallsBefore = mockedInvoke.mock.calls.filter(
      ([name]) => name === "model_config_read",
    ).length;
    const agentsCallsBefore = mockedInvoke.mock.calls.filter(
      ([name]) => name === "custom_instructions_read",
    ).length;
    await wrapper
      .find(
        ".settings-section-model-config .model-config-card .model-config-reload-btn",
      )
      .trigger("click");
    await flushPromises();
    await agentsCard.find(".model-config-reload-btn").trigger("click");
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

  it("保存模型配置成功后自动重读目录卡片", async () => {
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
      .find(
        ".settings-section-model-config .model-config-card .model-config-actions .model-config-save-btn",
      )
      .trigger("click");
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
    ).toBe('{\n  "models": [{ "id": "gpt-x" }]\n}');
    expect(store.toast).toContain("模型配置已保存");
  });

  it("config 未配置 model_catalog_json 时仍可编辑（默认 models.json）", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read")
        return Promise.resolve({
          ...sampleModelConfig,
          model_catalog_json: "",
          model_catalog_path: "C:/apps/codex-ui/.codex/models.json",
          model_catalog_exists: false,
          model_catalog: "",
        });
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    // 未配置时使用默认解析路径，编辑器仍可编辑
    expect(
      wrapper.find("textarea.model-config-textarea").attributes("disabled"),
    ).toBeUndefined();
    // 路径信息只通过标题链接的 tooltip 暴露（不再有独立路径行）
    const titleLink = wrapper.find(
      ".settings-section-model-config .model-catalog-head .model-config-title-link",
    );
    expect(titleLink.attributes("disabled")).toBeDefined();
    expect(
      wrapper.find(".model-catalog-title-wrap").attributes("data-tip"),
    ).toContain("models.json");
    expect(
      wrapper.find(".model-catalog-block .model-config-path").exists(),
    ).toBe(false);
    expect(
      wrapper.findAll(".settings-section-model-config .model-config-path-link")
        .length,
    ).toBe(0);
  });

  it("模型名称输入点击展开全部候选（由模型目录 slug 生成，不过滤）", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .find("textarea.model-config-textarea")
      .setValue(
        '{\n  "models": [\n    { "slug": "m1", "display_name": "M1" },\n    { "slug": "m2", "display_name": "M2" }\n  ]\n}',
      );
    await flushPromises();
    const input = wrapper.find("#model-config-ui-model");
    // datalist 已移除，改为点击输入框展开的自定义下拉（不再有按值过滤的 list 属性）
    expect(input.attributes("list")).toBeUndefined();
    await input.trigger("click");
    const opts = wrapper.findAll(".model-config-model-picker-menu .option-btn");
    expect(opts.map((o) => o.text())).toEqual(["m1", "m2"]);
    // 点击候选回填到模型名称
    await opts[0].trigger("click");
    expect(
      (wrapper.find("#model-config-ui-model").element as HTMLInputElement)
        .value,
    ).toBe("m1");
  });

  it("模型配置添加按钮 aria-label 为「添加模型提供方」", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    expect(wrapper.find(".model-config-add-btn").attributes("aria-label")).toBe(
      "添加模型提供方",
    );
  });

  it("渲染提供方列表与激活单选", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const rows = wrapper.findAll(
      ".model-provider-row:not(.model-provider-none)",
    );
    expect(rows.length).toBe(2);
    expect(rows[0].text()).toContain("DeepSeek");
    expect(rows[0].text()).toContain("deepseek");
    const radios = wrapper.findAll('input[name="model-provider-active"]');
    expect(radios.length).toBe(3);
    // 首项为固定的「不使用模型提供方」（value 为空），本例未激活，不选中
    expect((radios[0].element as HTMLInputElement).value).toBe("");
    expect((radios[0].element as HTMLInputElement).checked).toBe(false);
    expect((radios[1].element as HTMLInputElement).checked).toBe(true);
    expect((radios[2].element as HTMLInputElement).checked).toBe(false);
  });

  it("固定的「不使用模型提供方」项无编辑/删除按钮，选中表示不选提供方", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const noneRow = wrapper.find(".model-provider-row.model-provider-none");
    expect(noneRow.exists()).toBe(true);
    expect(noneRow.find(".provider-row-edit").exists()).toBe(false);
    expect(noneRow.find(".provider-row-delete").exists()).toBe(false);
    expect(noneRow.text()).toContain("不使用模型提供方");
    // 选中「不使用模型提供方」后 model_provider 为空
    const noneRadio = noneRow.find('input[name="model-provider-active"]');
    await noneRadio.setValue();
    expect((noneRadio.element as HTMLInputElement).checked).toBe(true);
    expect(
      (
        wrapper.find('input[name="model-provider-active"]')
          .element as HTMLInputElement
      ).value,
    ).toBe("");
  });

  it("选中「不使用模型提供方」保存：不删既有提供方，model_provider 写 null", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .find(
        '.model-provider-row.model-provider-none input[name="model-provider-active"]',
      )
      .setValue();
    await wrapper
      .find(
        ".settings-section-model-config .model-config-actions .model-config-save-btn",
      )
      .trigger("click");
    await flushPromises();

    const batchWrite = mockedInvoke.mock.calls.find(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method ===
          "config/batchWrite",
    );
    expect(batchWrite).toBeTruthy();
    const edits = (
      batchWrite![1] as {
        params: { edits: { keyPath: string; value: unknown }[] };
      }
    ).params.edits;
    // 空 provider 删键（写空串会让 codex 判配置无效 → 用户层消失 → 提供方被误删）
    expect(edits.find((e) => e.keyPath === "model_provider")?.value).toBeNull();
    // 提供方表仍按完整列表写入：既有 deepseek / other 都保留
    expect(
      Object.keys(
        edits.find((e) => e.keyPath === "model_providers")?.value as object,
      ),
    ).toEqual(["deepseek", "other"]);
    expect(store.toast).toContain("模型配置已保存");
  });

  it("历史 wire_api=chat：行内提示且保存被阻断", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        const read = structuredClone(sampleModelConfigRead);
        read.layers[0].config.model_providers.other.wire_api = "chat";
        return Promise.resolve(read);
      }
      if (cmd === "model_config_read")
        return Promise.resolve(sampleModelConfig);
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();

    expect(
      wrapper.find(".model-provider-row.model-provider-row-error").text(),
    ).toContain("wire_api 仅支持 responses");
    await wrapper
      .find(
        ".settings-section-model-config .model-config-actions .model-config-save-btn",
      )
      .trigger("click");
    await flushPromises();
    // 组件在保存前按行校验并阻断（提示即行内错误文案）
    expect(store.toast).toContain("wire_api 仅支持 responses");
    expect(
      mockedInvoke.mock.calls.some(
        ([cmd, args]) =>
          cmd === "codex_rpc" &&
          (args as { method?: string } | undefined)?.method ===
            "config/batchWrite",
      ),
    ).toBe(false);
    wrapper.unmount();
  });

  it("requires_openai_auth 提供者不报缺认证", async () => {
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        return Promise.resolve({
          ...sampleModelConfigRead,
          layers: [
            {
              name: { type: "user" },
              config: {
                model: "gpt-5.6-luna",
                model_reasoning_effort: "max",
                model_provider: "newapi",
                model_providers: {
                  newapi: {
                    name: "NewAPI",
                    base_url: "https://newpi.bond/v1",
                    wire_api: "responses",
                    requires_openai_auth: true,
                  },
                },
              },
            },
          ],
        });
      }
      if (cmd === "model_config_read")
        return Promise.resolve({
          ...sampleModelConfig,
          providers: [],
        });
      if (cmd === "custom_instructions_read")
        return Promise.resolve(sampleAgentsState);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    const row = wrapper.find(".model-provider-row:not(.model-provider-none)");
    expect(row.text()).toContain("NewAPI");
    expect(row.find(".model-provider-row-msg").exists()).toBe(false);
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
    expect(wrapper.text()).toContain("点击右上角「+」创建");
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".model-provider-form").exists()).toBe(true);
    await wrapper.find('input[placeholder="如 my-provider"]').setValue("first");
    await wrapper.find('input[placeholder="如 DeepSeek"]').setValue("First");
    await wrapper
      .find('input[placeholder="https://api.example.com/v1"]')
      .setValue("https://first.example.com/v1");
    await wrapper
      .find('input[placeholder="如 OPENAI_API_KEY"]')
      .setValue("MY_API_KEY");
    await wrapper.find(".provider-form-submit").trigger("click");
    await flushPromises();
    const radios = wrapper.findAll('input[name="model-provider-active"]');
    expect(radios.length).toBe(2);
    // 首项「不使用模型提供方」未选中，新增的 first 自动激活
    expect((radios[0].element as HTMLInputElement).checked).toBe(false);
    expect((radios[1].element as HTMLInputElement).checked).toBe(true);
  });

  it("重复提供方标识被拦截", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    await wrapper
      .find('input[placeholder="如 my-provider"]')
      .setValue("deepseek");
    await wrapper.find(".provider-form-submit").trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("提供方标识已存在");
    expect(
      wrapper.findAll(".model-provider-row:not(.model-provider-none)").length,
    ).toBe(2);
  });

  it("新增提供方以弹窗呈现：标题、遮罩与表单均在弹窗内", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal-mask").exists()).toBe(true);
    expect(wrapper.find(".modal-title").text()).toBe("添加模型提供方");
    expect(wrapper.find(".modal-body .model-provider-form").exists()).toBe(
      true,
    );
    expect(wrapper.find(".modal-foot .btn.primary").text()).toBe("确认");
  });

  it("编辑提供方以弹窗呈现：标题为编辑且标识只读", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".model-provider-row:not(.model-provider-none)")[1]
      .find(".provider-row-edit")
      .trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal-title").text()).toBe("编辑模型提供方");
    expect(
      wrapper
        .find('input[placeholder="如 my-provider"]')
        .attributes("disabled"),
    ).toBeDefined();
    expect(wrapper.find(".modal-foot .btn.primary").text()).toBe("确认");
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
    expect(
      wrapper.findAll(".model-provider-row:not(.model-provider-none)").length,
    ).toBe(2);
    // 重新打开表单复位
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    expect(
      (
        wrapper.find('input[placeholder="如 my-provider"]')
          .element as HTMLInputElement
      ).value,
    ).toBe("");
    // 取消关闭
    await wrapper.find(".modal-foot .btn:not(.primary)").trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal-mask").exists()).toBe(false);
  });

  it("编辑提供方后保存调用 config/batchWrite", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const rows = wrapper.findAll(
      ".model-provider-row:not(.model-provider-none)",
    );
    await rows[1].find(".provider-row-edit").trigger("click");
    await flushPromises();
    expect(wrapper.find(".model-provider-form").exists()).toBe(true);
    expect(
      wrapper
        .find('input[placeholder="如 my-provider"]')
        .attributes("disabled"),
    ).toBeDefined();
    await wrapper
      .find('input[placeholder="https://api.example.com/v1"]')
      .setValue("https://new.example.com/v1");
    await wrapper.find(".provider-form-submit").trigger("click");
    await flushPromises();
    await wrapper
      .find(
        ".settings-section-model-config .model-config-actions .model-config-save-btn",
      )!
      .trigger("click");
    await flushPromises();
    const saveCall = mockedInvoke.mock.calls.find(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method ===
          "config/batchWrite",
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
        wire_api: "responses",
      },
    });
    expect(edits[1]).toEqual({
      keyPath: "model_provider",
      value: "deepseek",
      mergeStrategy: "replace",
    });
    expect(store.toast).toContain("模型配置已保存");
  });

  it("激活项与非激活项都可删除，删除激活项后清空激活", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const rows = wrapper.findAll(
      ".model-provider-row:not(.model-provider-none)",
    );
    const delBtns = rows.map((r) => r.find(".provider-row-delete")!);
    expect(delBtns[0].attributes("disabled")).toBeUndefined();
    expect(delBtns[1].attributes("disabled")).toBeUndefined();
    expect(delBtns[1].classes()).toContain("danger");
    await delBtns[0].trigger("click");
    await flushPromises();
    expect(
      wrapper.findAll(".model-provider-row:not(.model-provider-none)").length,
    ).toBe(1);
  });

  it("不再展示固定值 UI", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    expect(wrapper.find(".model-config-fixed").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("固定值");
  });

  it("model 为空时允许保存并写入 config", async () => {
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
      .find(
        ".settings-section-model-config .model-config-actions .model-config-save-btn",
      )!
      .trigger("click");
    await flushPromises();
    expect(
      mockedInvoke.mock.calls.some(
        ([cmd, args]) =>
          cmd === "codex_rpc" &&
          (args as { method?: string } | undefined)?.method ===
            "config/batchWrite",
      ),
    ).toBe(true);
  });

  it("认证两字段默认不写入，分别选择后各自写入", async () => {
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
        .find(
          ".settings-section-model-config .model-config-actions .model-config-save-btn",
        )!
        .trigger("click");
    const lastAuthEdits = () => {
      const calls = mockedInvoke.mock.calls.filter(
        ([cmd, args]) =>
          cmd === "codex_rpc" &&
          (args as { method?: string } | undefined)?.method ===
            "config/batchWrite",
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
    ).toBe(null);
    expect(edits.find((e) => e.keyPath === "forced_login_method")?.value).toBe(
      null,
    );

    await pickAppSelect(wrapper, "model-config-ui-auth", "apikey");
    await pickAppSelect(wrapper, "model-config-ui-forced", "api");
    await save();
    await flushPromises();
    edits = lastAuthEdits();
    expect(
      edits.find((e) => e.keyPath === "preferred_auth_method")?.value,
    ).toBe("apikey");
    expect(edits.find((e) => e.keyPath === "forced_login_method")?.value).toBe(
      "api",
    );
  });

  it("模型目录内容非法 JSON 时提示并阻断保存（不写 config）", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find("textarea.model-config-textarea").setValue("{ not json");
    const batchCallsBefore = mockedInvoke.mock.calls.filter(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method ===
          "config/batchWrite",
    ).length;
    await wrapper
      .find(
        ".settings-section-model-config .model-config-card .model-config-actions .model-config-save-btn",
      )
      .trigger("click");
    await flushPromises();
    // 红框行内错误，且未落盘 config
    expect(
      wrapper.find(".model-catalog-block .model-config-field-error").text(),
    ).toContain("模型目录不是合法 JSON");
    expect(mockedInvoke).not.toHaveBeenCalledWith("model_catalog_save", {
      content: "{ not json",
    });
    expect(
      mockedInvoke.mock.calls.filter(
        ([cmd, args]) =>
          cmd === "codex_rpc" &&
          (args as { method?: string } | undefined)?.method ===
            "config/batchWrite",
      ).length,
    ).toBe(batchCallsBefore);
  });

  it("模型目录为空时保存删除 config 的 model_catalog_json 键", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find("textarea.model-config-textarea").setValue("   ");
    await wrapper
      .find(
        ".settings-section-model-config .model-config-card .model-config-actions .model-config-save-btn",
      )
      .trigger("click");
    await flushPromises();
    expect(mockedInvoke).not.toHaveBeenCalledWith("model_catalog_save", {
      content: "   ",
    });
    const saveCall = mockedInvoke.mock.calls.find(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method ===
          "config/batchWrite",
    );
    expect(saveCall).toBeTruthy();
    const edits = (
      saveCall![1] as {
        params: { edits: { keyPath: string; value: unknown }[] };
      }
    ).params.edits;
    expect(
      edits.find((e) => e.keyPath === "model_catalog_json")?.value,
    ).toBeNull();
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
    await wrapper.find('input[placeholder="如 DeepSeek"]').setValue("Env Only");
    await wrapper
      .find('input[placeholder="https://api.example.com/v1"]')
      .setValue("https://env.example.com/v1");
    await flushPromises();
    // 全局 OPENAI_API_KEY 存在时认证可省略；行下不再显示额外提示
    expect(wrapper.text()).not.toContain("已检测到全局 OPENAI_API_KEY");
    await wrapper.find(".provider-form-submit").trigger("click");
    await flushPromises();
    expect(
      wrapper.findAll(".model-provider-row:not(.model-provider-none)").length,
    ).toBe(1);
  });

  it("提供方表单缺少名称/base_url/认证时逐字段提示", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    const add = () => wrapper.find(".provider-form-submit").trigger("click");
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
    expect(wrapper.text()).toContain(
      "请填写 env_key、API Key 或选择 ChatGPT/OpenAI 登录认证",
    );
    expect(
      wrapper.findAll(".model-provider-row:not(.model-provider-none)").length,
    ).toBe(2);
  });

  it("勾选 ChatGPT/OpenAI 登录认证后隐藏 env_key / API Key 字段，取消恢复", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".model-config-add-btn").trigger("click");
    await flushPromises();
    const envInput = () =>
      wrapper.find('input[placeholder="如 OPENAI_API_KEY"]');
    const tokenInput = () => wrapper.find('input[placeholder="API Key"]');
    expect(envInput().exists()).toBe(true);
    expect(tokenInput().exists()).toBe(true);
    await wrapper.find("#provider-openai-auth").setValue(true);
    await flushPromises();
    expect(envInput().exists()).toBe(false);
    expect(tokenInput().exists()).toBe(false);
    await wrapper.find("#provider-openai-auth").setValue(false);
    await flushPromises();
    expect(envInput().exists()).toBe(true);
    expect(tokenInput().exists()).toBe(true);
  });

  it("勾选 ChatGPT/OpenAI 登录认证并保存后清空 env_key / experimental_bearer_token", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const rows = wrapper.findAll(
      ".model-provider-row:not(.model-provider-none)",
    );
    await rows[1].find(".provider-row-edit").trigger("click");
    await flushPromises();
    await wrapper.find("#provider-openai-auth").setValue(true);
    await flushPromises();
    await wrapper.find(".provider-form-submit").trigger("click");
    await flushPromises();
    await wrapper
      .find(
        ".settings-section-model-config .model-config-actions .model-config-save-btn",
      )!
      .trigger("click");
    await flushPromises();
    const saveCall = mockedInvoke.mock.calls.find(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method ===
          "config/batchWrite",
    );
    expect(saveCall).toBeTruthy();
    const edits = (
      saveCall![1] as {
        params: { edits: { keyPath: string; value: unknown }[] };
      }
    ).params.edits;
    const providers = edits[0].value as Record<string, Record<string, unknown>>;
    expect(edits[0].keyPath).toBe("model_providers");
    expect(providers.other).toEqual({
      name: "Other",
      base_url: "https://other.example.com/v1",
      wire_api: "responses",
      requires_openai_auth: true,
    });
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

describe("SettingsView 动态工具", () => {
  beforeEach(() => {
    __resetTabsForTest();
    store.toast = "";
    mockedInvoke.mockReset();
    mockedSave.mockReset();
    // 模拟真实 saveSettings：合并 patch 回 store.settings，保证开关 reactive 依赖生效
    mockedSave.mockImplementation(async (patch) => {
      store.settings = { ...store.settings, ...patch };
    });
    (mockedInvoke as ReturnType<typeof vi.fn>).mockImplementation(
      async () => undefined,
    );
    store.settings.dynamic_tools_disabled = [];
  });

  it("导航位于模型配置与技能管理之间，渲染标题与四个工具行", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const nav = wrapper.findAll(".settings-nav-item");
    const idx = (t: string) => nav.map((i) => i.text().trim()).indexOf(t);
    expect(idx("模型快照")).toBe(3);
    expect(idx("模型配置")).toBe(4);
    expect(idx("动态工具")).toBe(5);
    expect(idx("技能管理")).toBe(6);
    expect(idx("MCP管理")).toBe(7);
    expect(idx("插件管理")).toBe(8);
    expect(idx("定时任务")).toBe(9);
    await nav[5].trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("动态工具");
    expect(wrapper.text()).toContain("禁用后新会话不再注入");
    const rows = wrapper.findAll(".dynamic-tool-row");
    expect(rows).toHaveLength(4);
    expect(rows[0].text()).toContain("codexui_get_usage");
    expect(rows[0].text()).toContain("查询当前会话的 token 消耗");
    expect(rows[1].text()).toContain("codexui_compact_context");
    expect(rows[2].text()).toContain("codexui_add_scheduled_task");
    expect(rows[3].text()).toContain("codexui_send_file_to_wechat");
  });

  it("切换开关写入 dynamic_tools_disabled（禁用后不再注入）", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    const nav = wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("动态工具"))!;
    await nav.trigger("click");
    await flushPromises();

    const firstSwitch = wrapper.findAll(
      ".dynamic-tool-row input[type='checkbox']",
    )[0];
    await firstSwitch.setValue(false);
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith({
      dynamic_tools_disabled: ["codexui.get_usage"],
    });

    const disabledResult = wrapper.findAll(
      ".dynamic-tool-row input[type='checkbox']",
    )[0];
    await disabledResult.setValue(true);
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith({ dynamic_tools_disabled: [] });

    wrapper.unmount();
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

  it("禁用技能 switch 开启后写配置并强制刷新", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    const rows = wrapper.findAll(".skill-row");
    expect(
      (rows[1].find(".skill-actions .switch input").element as HTMLInputElement)
        .checked,
    ).toBe(false);
    await rows[1].find(".skill-actions .switch input").setValue(true);
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

  it("启用状态技能 switch 关闭后写禁用配置", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    const rows = wrapper.findAll(".skill-row");
    expect(
      (rows[0].find(".skill-actions .switch input").element as HTMLInputElement)
        .checked,
    ).toBe(true);
    await rows[0].find(".skill-actions .switch input").setValue(false);
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
    await rows[1].find(".skill-actions .switch input").setValue(true);
    await flushPromises();
    expect(store.toast).toContain("配置写入失败");
    const forcedReloads = mockedInvoke.mock.calls.filter(
      ([name, arg]) =>
        name === "skills_read" &&
        (arg as Record<string, unknown> | undefined)?.forceReload === true,
    );
    expect(forcedReloads.length).toBe(0);
  });

  it("技能管理头部不显示数量文本并提供添加按钮", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    const head = wrapper.find(
      ".settings-section-skills .model-config-card-head",
    );
    expect(head.text()).not.toContain("共");
    expect(head.text()).not.toContain("个");
    const addBtn = head.find(".skill-add-btn");
    expect(addBtn.exists()).toBe(true);
    expect(addBtn.attributes("aria-label")).toBe("添加技能");
  });

  it("添加技能成功：toast 技能名并强制刷新列表", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "skills_read") return Promise.resolve(sampleSkillsState);
      if (cmd === "skills_add") return Promise.resolve("new-skill");
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    const before = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    await wrapper.find(".skill-add-btn").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("skills_add");
    expect(store.toast).toContain("已添加技能 new-skill");
    const after = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    expect(after).toBe(before + 1);
    expect(mockedInvoke.mock.calls[mockedInvoke.mock.calls.length - 1]).toEqual(
      ["skills_read", { forceReload: true }],
    );
  });

  it("添加技能取消（null）：不 toast、不刷新", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "skills_read") return Promise.resolve(sampleSkillsState);
      if (cmd === "skills_add") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    const before = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    await wrapper.find(".skill-add-btn").trigger("click");
    await flushPromises();
    const after = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    expect(after).toBe(before);
    expect(store.toast).not.toContain("已添加技能");
  });

  it("添加技能失败：toast 错误且不刷新", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "skills_read") return Promise.resolve(sampleSkillsState);
      if (cmd === "skills_add") throw new Error("校验失败：文件夹名不一致");
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    const before = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    await wrapper.find(".skill-add-btn").trigger("click");
    await flushPromises();
    expect(store.toast).toContain("校验失败：文件夹名不一致");
    const after = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    expect(after).toBe(before);
  });

  it("技能管理每行提供删除按钮；确认后调用 skills_remove 并强制刷新", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "skills_read") return Promise.resolve(sampleSkillsState);
      if (cmd === "skills_remove") return Promise.resolve("pdf");
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    const row = wrapper.find(".skill-row");
    const deleteBtn = row.find(".skill-delete-btn");
    expect(deleteBtn.exists()).toBe(true);
    expect(deleteBtn.attributes("aria-label")).toBe("删除技能");
    const before = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    await deleteBtn.trigger("click");
    await flushPromises();
    settleConfirm(true);
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("skills_remove", {
      skillPath: "C:/apps/codex-ui/.codex/skills/pdf/SKILL.md",
    });
    expect(store.toast).toContain("已删除技能 pdf");
    const after = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    expect(after).toBe(before + 1);
  });

  it("删除技能取消：不调用 skills_remove、不刷新", async () => {
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    const before = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    await wrapper.find(".skill-delete-btn").trigger("click");
    await flushPromises();
    settleConfirm(false);
    await flushPromises();
    expect(
      mockedInvoke.mock.calls.filter(([name]) => name === "skills_remove")
        .length,
    ).toBe(0);
    const after = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    expect(after).toBe(before);
    expect(store.toast).not.toContain("已删除技能");
  });

  it("删除技能失败：toast 错误且不刷新", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "skills_read") return Promise.resolve(sampleSkillsState);
      if (cmd === "skills_remove") throw new Error("删除失败：目录不存在");
      return Promise.resolve(undefined);
    });
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("技能管理"))!
      .trigger("click");
    await flushPromises();
    const before = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    await wrapper.find(".skill-delete-btn").trigger("click");
    await flushPromises();
    settleConfirm(true);
    await flushPromises();
    expect(store.toast).toContain("删除失败：目录不存在");
    const after = mockedInvoke.mock.calls.filter(
      ([name]) => name === "skills_read",
    ).length;
    expect(after).toBe(before);
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
    expect(wrapper.findAll(".mcp-server-type").map((t) => t.text())).toEqual([
      "stdio",
      "http",
    ]);
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
        wrapper.find(
          '.mcp-server-form .mcp-omit-option input[value="deferred"]',
        ).element as HTMLInputElement
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
      .find('.mcp-server-form input[placeholder^="服务器进程启动目录"]')
      .setValue("D:\\mcp-servers");
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
                cwd: "D:\\mcp-servers",
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
    await pickAppSelect(wrapper, "mcp-form-transport", "Streamable HTTP");
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
    await pickAppSelect(wrapper, "mcp-form-transport", "Streamable HTTP");
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
      wrapper.find("#mcp-form-transport .app-select-label").text().trim(),
    ).toBe("stdio");
    expect(
      (
        wrapper.find('.mcp-server-form input[placeholder="如 filesystem"]')
          .element as HTMLInputElement
      ).value,
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
        (args as { method?: string } | undefined)?.method ===
          "config/batchWrite",
    ) as [
      string,
      {
        params: { edits: { value: Record<string, Record<string, unknown>> }[] };
      },
    ];
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
              name: {
                type: "user",
                file: "C:/x/.codex/config.toml",
                profile: null,
              },
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
        wrapper.find(
          '.mcp-server-form .mcp-omit-option input[value="code_mode"]',
        ).element as HTMLInputElement
      ).checked,
    ).toBe(true);
    expect(
      (
        wrapper.find(
          '.mcp-server-form .mcp-omit-option input[value="deferred"]',
        ).element as HTMLInputElement
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
        wrapper.find(
          '.mcp-server-form .mcp-omit-option input[value="deferred"]',
        ).element as HTMLInputElement
      ).checked,
    ).toBe(false);
    expect(
      (
        wrapper.find(
          '.mcp-server-form .mcp-omit-option input[value="code_mode"]',
        ).element as HTMLInputElement
      ).checked,
    ).toBe(false);
  });

  /** mcpServerStatus/list 样本：filesystem 已就绪并带工具/资源/模板 */
  const sampleMcpStatusList = {
    data: [
      {
        name: "filesystem",
        pluginId: null,
        serverInfo: {
          title: "Filesystem Server",
          version: "1.0.0",
          description: "本地文件访问服务器",
          websiteUrl: "https://example.com/fs",
          icons: null,
        },
        tools: {
          read_file: {
            name: "read_file",
            title: "读取文件",
            description: "读取指定路径的文本文件",
            inputSchema: {
              type: "object",
              properties: { path: { type: "string" } },
            },
          },
          write_file: {
            name: "write_file",
            description: "写入文件",
            inputSchema: { type: "object" },
          },
        },
        resources: [
          {
            uri: "file:///data/a.txt",
            name: "a.txt",
            description: "示例数据",
            mimeType: "text/plain",
          },
        ],
        resourceTemplates: [
          {
            uriTemplate: "file:///data/{name}",
            name: "data",
            description: "数据目录文件",
          },
        ],
        authStatus: "bearerToken",
      },
    ],
  };

  /** 在当前 mock 之上叠加 mcpServerStatus/list 的应答 */
  function stubMcpStatus(list = sampleMcpStatusList) {
    const baseImpl = mockedInvoke.getMockImplementation()!;
    mockedInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "mcpServerStatus/list")
        return Promise.resolve(list);
      return baseImpl(cmd, args);
    });
  }

  it("MCP 行信息按钮打开详情弹窗，默认服务器信息 Tab 展示配置与元数据", async () => {
    stubMcpStatus();
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    const rows = wrapper.findAll(".mcp-server-row");
    expect(rows[0].find(".mcp-row-info").attributes("aria-label")).toBe(
      "查看详情",
    );
    await rows[0].find(".mcp-row-info").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "mcpServerStatus/list",
      params: { detail: "full" },
    });
    expect(wrapper.find(".modal-title").text()).toBe("filesystem");
    expect(wrapper.findAll(".mcp-detail-tab")).toHaveLength(3);
    expect(
      wrapper.findAll(".mcp-detail-tab")[0].attributes("aria-selected"),
    ).toBe("true");
    const fields = wrapper.find(".mcp-detail-fields");
    expect(fields.text()).toContain("filesystem");
    expect(fields.text()).toContain("stdio");
    expect(fields.text()).toContain("npx");
    expect(fields.text()).toContain("Filesystem Server");
    expect(fields.text()).toContain("1.0.0");
    expect(fields.text()).toContain("Bearer Token");
    await wrapper.find(".modal-close").trigger("click");
    await flushPromises();
    expect(wrapper.find(".mcp-detail").exists()).toBe(false);
  });

  it("MCP 详情工具 Tab 折叠展示输入参数 JSON 且带数量角标", async () => {
    stubMcpStatus();
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    await wrapper
      .findAll(".mcp-server-row")[0]
      .find(".mcp-row-info")
      .trigger("click");
    await flushPromises();
    const tabs = wrapper.findAll(".mcp-detail-tab");
    expect(tabs[1].text()).toContain("工具");
    expect(tabs[1].find(".mcp-detail-tab-count").text()).toBe("2");
    await tabs[1].trigger("click");
    await flushPromises();
    expect(
      wrapper.findAll(".mcp-detail-tab")[1].attributes("aria-selected"),
    ).toBe("true");
    expect(wrapper.find(".mcp-detail-tools").text()).toContain("read_file");
    expect(wrapper.find(".mcp-detail-tools").text()).toContain(
      "读取指定路径的文本文件",
    );
    expect(wrapper.findAll(".mcp-detail-schema")).toHaveLength(2);
    expect(wrapper.find(".mcp-detail-schema pre").text()).toContain('"path"');
  });

  it("MCP 详情资源 Tab：分组渲染并复制 URI", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    const clipboardDesc = Object.getOwnPropertyDescriptor(
      navigator,
      "clipboard",
    );
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    stubMcpStatus();
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    await wrapper
      .findAll(".mcp-server-row")[0]
      .find(".mcp-row-info")
      .trigger("click");
    await flushPromises();
    const tabs = wrapper.findAll(".mcp-detail-tab");
    expect(tabs[2].find(".mcp-detail-tab-count").text()).toBe("2");
    await tabs[2].trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("资源（1）");
    expect(wrapper.text()).toContain("资源模板（1）");
    expect(wrapper.text()).toContain("file:///data/a.txt");
    await wrapper.find(".mcp-resource-copy-btn").trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenCalledWith("file:///data/a.txt");
    expect(store.toast).toContain("URI 已复制");
    if (clipboardDesc) {
      Object.defineProperty(navigator, "clipboard", clipboardDesc);
    } else {
      Reflect.deleteProperty(navigator, "clipboard");
    }
  });

  it("MCP 详情未查询到状态时展示空态并可重新查询", async () => {
    stubMcpStatus({ data: [] });
    const wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper
      .findAll(".settings-nav-item")
      .find((i) => i.text().includes("MCP管理"))!
      .trigger("click");
    await flushPromises();
    await wrapper
      .findAll(".mcp-server-row")[0]
      .find(".mcp-row-info")
      .trigger("click");
    await flushPromises();
    expect(wrapper.find(".mcp-detail-empty").text()).toContain("未查询到");
    const before = mockedInvoke.mock.calls.filter(
      ([name, arg]) =>
        name === "codex_rpc" &&
        (arg as Record<string, unknown> | undefined)?.method ===
          "mcpServerStatus/list",
    ).length;
    await wrapper.find(".mcp-detail-empty .btn").trigger("click");
    await flushPromises();
    const after = mockedInvoke.mock.calls.filter(
      ([name, arg]) =>
        name === "codex_rpc" &&
        (arg as Record<string, unknown> | undefined)?.method ===
          "mcpServerStatus/list",
    ).length;
    expect(after).toBe(before + 1);
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
    mockedGetVersion.mockReset();
    mockedGetVersion.mockResolvedValue("1.0.0");
    store.server.codexVersion = "0.154.0";
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

  it("个性化 / 全局指令 / 模型配置 / 技能管理 / MCP管理 / 插件管理六个分区均渲染", () => {
    wrapper = mount(SettingsView);
    const titles = wrapper
      .findAll(".settings-section-title")
      .map((s) => s.text());
    expect(titles).toContain("个性化");
    expect(titles).toContain("基础设置");
    expect(titles).toContain("全局指令");
    expect(titles).toContain("模型配置");
    expect(titles).toContain("技能管理");
    expect(titles).toContain("MCP 管理");
    expect(titles).toContain("插件管理");
  });

  it("左侧导航渲染十二个分类，默认选中第一个", () => {
    wrapper = mount(SettingsView);
    const items = wrapper.findAll(".settings-nav-item");
    expect(items.map((i) => i.text().trim())).toEqual([
      "个性化",
      "基础设置",
      "全局指令",
      "模型快照",
      "模型配置",
      "动态工具",
      "技能管理",
      "MCP管理",
      "插件管理",
      "定时任务",
      "兼容代理",
      "关于",
    ]);
    expect(items[0].classes()).toContain("active");
    expect(items[1].classes()).not.toContain("active");
    expect(items[2].classes()).not.toContain("active");
    expect(items[3].classes()).not.toContain("active");
    expect(items[4].classes()).not.toContain("active");
    expect(items[5].classes()).not.toContain("active");
    expect(items[6].classes()).not.toContain("active");
    expect(items[7].classes()).not.toContain("active");
    expect(items[8].classes()).not.toContain("active");
    expect(items[9].classes()).not.toContain("active");
    expect(items[10].classes()).not.toContain("active");
    expect(items[11].classes()).not.toContain("active");
    const personal = wrapper.find(".settings-section-personalization")
      .element as HTMLElement;
    const globalInstructions = wrapper.find(
      ".settings-section-global-instructions",
    ).element as HTMLElement;
    const skills = wrapper.find(".settings-section-skills")
      .element as HTMLElement;
    const mcp = wrapper.find(".settings-section-mcp").element as HTMLElement;
    const plugins = wrapper.find(".settings-section-plugins")
      .element as HTMLElement;
    expect(personal.style.display).not.toBe("none");
    expect(globalInstructions.style.display).toBe("none");
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
    const globalInstructions = wrapper.find(
      ".settings-section-global-instructions",
    ).element as HTMLElement;
    const plugins = wrapper.find(".settings-section-plugins")
      .element as HTMLElement;
    expect(globalInstructions.style.display).toBe("none");
    expect(plugins.style.display).not.toBe("none");
  });

  it("「关于」分区渲染应用信息，点击导航切换", async () => {
    wrapper = mount(SettingsView);
    await flushPromises();
    const about = wrapper.find(".settings-section-about");
    expect(about.exists()).toBe(true);
    expect(about.find(".model-config-card-head h3").text()).toBe("Codex UI");
    expect(about.find(".about-tagline").text()).toBe(
      "由 xljiulang 100% vibe coding 而成",
    );
    expect(about.find(".about-value").text()).toBe("1.0.0");
    const items = wrapper.findAll(".settings-nav-item");
    const aboutItem = items.find((i) => i.text().includes("关于"))!;
    expect(aboutItem.classes()).not.toContain("active");
    await aboutItem.trigger("click");
    expect(aboutItem.classes()).toContain("active");
    expect((about.element as HTMLElement).style.display).not.toBe("none");
  });

  it("「关于」按顺序显示四行，微信接入行取后端的协议名与协议版本", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "wechat_protocol_info") {
        return Promise.resolve({
          protocol: "ilink bot API",
          channelVersion: "1.1.0",
        });
      }
      return Promise.resolve(undefined);
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    const about = wrapper.find(".settings-section-about");
    expect(about.findAll(".about-row label").map((l) => l.text())).toEqual([
      "应用版本",
      "codex CLI 版本",
      "微信接入",
      "技术栈",
    ]);
    expect(about.findAll(".about-value")[2].text()).toBe(
      "ilink bot API · 协议版本 1.1.0（ClawBot 通道）",
    );
  });

  it("微信协议信息取不到时「关于」显示占位", async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "wechat_protocol_info") {
        return Promise.reject(new Error("no ipc"));
      }
      return Promise.resolve(undefined);
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    expect(
      wrapper.findAll(".settings-section-about .about-value")[2].text(),
    ).toBe("未知");
  });

  it("codex CLI 版本未知时「关于」显示占位", async () => {
    store.server.codexVersion = null;
    wrapper = mount(SettingsView);
    await flushPromises();
    const values = wrapper
      .findAll(".settings-section-about .about-value")
      .map((v) => v.text());
    expect(values[1]).toBe("未知版本");
  });
});

describe("SettingsView 毛玻璃主题外观开关", () => {
  beforeEach(() => {
    __resetTabsForTest();
    store.settings.glass_effect = true;
    store.toast = "";
    mockedSave.mockReset();
    mockedSave.mockResolvedValue(undefined);
  });

  it("主题行头部渲染「毛玻璃主题外观」与开关且默认开启", () => {
    const wrapper = mount(SettingsView);
    const input = wrapper.find("#glass").element as HTMLInputElement;
    expect(input.checked).toBe(true);
    const head = wrapper.find(
      ".settings-section-personalization .theme-row-head",
    );
    expect(head.find(".switch-text").text()).toBe("毛玻璃主题外观");
    expect(head.find(".switch input").attributes("id")).toBe("glass");
    // 文字在左、开关在右：头部第一个 label 是文字标签，开关紧随其后
    const labels = head.findAll("label");
    expect(labels[0].classes()).toContain("switch-text");
    expect(labels[1].classes()).toContain("switch");
  });

  it("个性化三行已改 switch，且毛玻璃开关不在其行内", () => {
    const wrapper = mount(SettingsView);
    const rows = wrapper.findAll(
      ".settings-section-personalization .setting-row.switch-row",
    );
    expect(rows).toHaveLength(3);
    // 文字在左、开关在右：switch 里的 input 与文字标签通过 id/for 关联（点文字也能切换）
    expect(rows[0].find(".switch > input").attributes("id")).toBe(
      "error-notify",
    );
    expect(rows[1].find(".switch > input").attributes("id")).toBe(
      "interaction-notify",
    );
    expect(rows[2].find(".switch > input").attributes("id")).toBe("enter");
    expect(rows[0].find(".switch-text").attributes("for")).toBe("error-notify");
    expect(rows[1].find(".switch-text").attributes("for")).toBe(
      "interaction-notify",
    );
    expect(rows[2].find(".switch-text").attributes("for")).toBe("enter");
    // 个性化里不再有原生复选框行
    expect(
      wrapper
        .find(".settings-section-personalization .setting-row.checkbox-row")
        .exists(),
    ).toBe(false);
    expect(wrapper.find("#glass").element.closest(".switch-row")).toBeNull();
  });

  it("取消勾选后即时保存 glass_effect=false", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find("#glass").setValue(false);
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith({
      glass_effect: false,
    });
  });

  it("个性化三行 switch 切换后即时保存", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find("#error-notify").setValue(false);
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith({ error_notify_enabled: false });

    await wrapper.find("#interaction-notify").setValue(false);
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith({
      interaction_notify_enabled: false,
    });

    await wrapper.find("#enter").setValue(false);
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith({ enter_to_send: false });
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

  it("渲染默认权限下拉并选中已保存值", async () => {
    const wrapper = mount(SettingsView);
    const trigger = wrapper.find("button.default-permission-select");
    expect(trigger.exists()).toBe(true);
    // AppSelect 弹层 Teleport 到 body：展开后从 document 读取选项
    await trigger.trigger("click");
    await nextTick();
    const menu = document.body.querySelector(".app-select-menu");
    expect(menu).toBeTruthy();
    const opts = Array.from(
      menu!.querySelectorAll<HTMLElement>(".app-select-option"),
    );
    expect(opts.map((o) => o.textContent?.trim())).toEqual(
      expect.arrayContaining(["只读访问", "请求批准", "帮我批准", "完全访问"]),
    );
    const selected = opts.find((o) => o.classList.contains("selected"));
    expect(selected?.textContent?.trim()).toBe("请求批准");
    wrapper.unmount();
  });

  it("切换默认权限后立即保存", async () => {
    const wrapper = mount(SettingsView);
    await wrapper.find("button.default-permission-select").trigger("click");
    await nextTick();
    const target = Array.from(
      document.body.querySelectorAll<HTMLElement>(".app-select-option"),
    ).find((o) => o.textContent?.trim() === "完全访问");
    expect(target).toBeTruthy();
    target!.click();
    await flushPromises();
    expect(mockedSave).toHaveBeenCalledWith(
      expect.objectContaining({ default_permission: "full-access" }),
    );
    wrapper.unmount();
  });
});

describe("SettingsView 基础设置", () => {
  let wrapper: ReturnType<typeof mount> | undefined;

  beforeEach(() => {
    __resetTabsForTest();
    __resetSessionTabsForTest();
    tabs.push(makeSessionTab("s1", "t1"));
    activeTabId.value = "s1";
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

  /** 点击左侧导航进入「基础设置」标签 */
  function openMemory(w: ReturnType<typeof mount>) {
    return w
      .findAll(".settings-nav-item")
      .find((i) => i.text().trim() === "基础设置")!
      .trigger("click");
  }

  /** config/read 返回用户层 [features]/[memories] */
  function mockMemoryConfig(enable: boolean, allowToolGenerate: boolean) {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        return {
          layers: [
            {
              name: { type: "user" },
              config: {
                features: { memories: enable },
                memories: {
                  use_memories: enable,
                  generate_memories: enable,
                  disable_on_external_context: !allowToolGenerate,
                },
              },
            },
          ],
        };
      }
      return {};
    });
  }

  it("切换到基础设置标签：从配置回填两个开关", async () => {
    mockMemoryConfig(true, false);
    wrapper = mount(SettingsView);
    await openMemory(wrapper);
    await flushPromises();
    const section = wrapper.find(".settings-section-basic");
    expect(section.exists()).toBe(true);
    const inputs = section.findAll(".switch input");
    expect(inputs).toHaveLength(2);
    expect((inputs[0].element as HTMLInputElement).checked).toBe(true);
    expect((inputs[1].element as HTMLInputElement).checked).toBe(false);
  });

  it("切换启用本地记忆：config/batchWrite 写 features.memories 与 use_memories", async () => {
    mockMemoryConfig(true, false);
    wrapper = mount(SettingsView);
    await openMemory(wrapper);
    await flushPromises();
    const inputs = wrapper.findAll(".settings-section-basic .switch input");
    await inputs[0].setValue(false);
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "config/batchWrite",
      params: {
        edits: [
          {
            keyPath: "features",
            value: expect.objectContaining({ memories: false }),
            mergeStrategy: "replace",
          },
          {
            keyPath: "memories",
            value: expect.objectContaining({
              use_memories: false,
              generate_memories: false,
              disable_on_external_context: true,
            }),
            mergeStrategy: "replace",
          },
        ],
        reloadUserConfig: true,
      },
    });
  });

  it("切换工具辅助生成：config/batchWrite 写 generate_memories", async () => {
    mockMemoryConfig(true, false);
    wrapper = mount(SettingsView);
    await openMemory(wrapper);
    await flushPromises();
    const inputs = wrapper.findAll(".settings-section-basic .switch input");
    await inputs[1].setValue(true);
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith(
      "codex_rpc",
      expect.objectContaining({
        method: "config/batchWrite",
        params: expect.objectContaining({
          edits: expect.arrayContaining([
            expect.objectContaining({
              keyPath: "memories",
              value: expect.objectContaining({
                generate_memories: true,
                use_memories: true,
                disable_on_external_context: false,
              }),
            }),
          ]),
        }),
      }),
    );
  });

  it("主开关关闭时，工具辅助开关禁用", async () => {
    mockMemoryConfig(false, false);
    wrapper = mount(SettingsView);
    await openMemory(wrapper);
    await flushPromises();
    const inputs = wrapper.findAll(".settings-section-basic .switch input");
    expect((inputs[1].element as HTMLInputElement).disabled).toBe(true);
  });

  it("删除本地记忆：确认后调用 memory/reset 并提示", async () => {
    wrapper = mount(SettingsView);
    await openMemory(wrapper);
    await flushPromises();
    await wrapper.find(".memory-delete-btn").trigger("click");
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
    expect(store.toast).toContain("记忆已删除");
  });

  it("删除本地记忆：取消时不调用 memory/reset", async () => {
    wrapper = mount(SettingsView);
    await openMemory(wrapper);
    await flushPromises();
    await wrapper.find(".memory-delete-btn").trigger("click");
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

  it("渲染终端 Shell 下拉，默认选中 cmd", async () => {
    wrapper = mount(SettingsView);
    const trigger = wrapper.find("button.terminal-shell-select");
    expect(trigger.exists()).toBe(true);
    await trigger.trigger("click");
    await nextTick();
    const menu = document.body.querySelector(".app-select-menu");
    expect(menu).toBeTruthy();
    const opts = Array.from(
      menu!.querySelectorAll<HTMLElement>(".app-select-option"),
    );
    expect(opts.map((o) => o.textContent?.trim())).toEqual([
      "cmd（命令提示符）",
      "PowerShell",
    ]);
    const selected = opts.find((o) => o.classList.contains("selected"));
    expect(selected?.textContent?.trim()).toBe("cmd（命令提示符）");
  });

  it("codex 可执行文件行位于基础设置分区内第一项", () => {
    wrapper = mount(SettingsView);
    const section = wrapper.find(".settings-section-basic");
    const labels = section
      .findAll(".settings .setting-row label")
      .map((l) => l.text());
    expect(labels[0]).toBe("codex 可执行文件（留空自动查找）");
    expect(labels[1]).toBe("终端 Shell");
  });

  it("切换 PowerShell 后立即保存", async () => {
    wrapper = mount(SettingsView);
    await wrapper.find("button.terminal-shell-select").trigger("click");
    await nextTick();
    const target = Array.from(
      document.body.querySelectorAll<HTMLElement>(".app-select-option"),
    ).find((o) => o.textContent?.trim() === "PowerShell");
    expect(target).toBeTruthy();
    target!.click();
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
              interface: {
                displayName: "Browser",
                shortDescription: "浏览器控制",
              },
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
    // 默认折叠：市场内的插件列表隐藏（已安装插件卡片常显，不在此断言范围）
    expect(mps[0].find(".plugin-list").exists()).toBe(false);
    expect(mps[0].text()).not.toContain("Browser");
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

  it("已安装插件卡片仅汇总 installed 插件并显示来源市场", async () => {
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
              version: "1.2.0",
              interface: {
                displayName: "Browser",
                shortDescription: "浏览器控制",
              },
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
              installed: true,
              enabled: false,
              interface: { displayName: "Gmail" },
            },
          ],
        },
      ],
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    const section = wrapper.find(".settings-section-plugins");
    const card = section.findAll(".model-config-card")[0];
    // 卡片位于插件市场卡片之前
    expect(card.find("h3").text()).toBe("已安装插件");
    const rows = card.findAll(".plugin-row");
    expect(rows.length).toBe(2);
    expect(rows[0].text()).toContain("Browser");
    expect(rows[0].text()).toContain("1.2.0");
    expect(rows[0].text()).toContain("来源：openai-bundled");
    expect(rows[1].text()).toContain("Gmail");
    expect(rows[1].text()).toContain("来源：openai-curated");
    // 未安装的 PDF 不出现在已安装卡片
    expect(card.text()).not.toContain("PDF");
    // 行内仅卸载按钮（无安装按钮）
    expect(card.findAll(".plugin-uninstall-btn").length).toBe(2);
    expect(card.find(".plugin-install-btn").exists()).toBe(false);
  });

  it("无已安装插件时已安装卡片显示空态", async () => {
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
              interface: { displayName: "PDF" },
            },
          ],
        },
      ],
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    const card = wrapper
      .find(".settings-section-plugins")
      .findAll(".model-config-card")[0];
    expect(card.find("h3").text()).toBe("已安装插件");
    expect(card.text()).toContain("还没有已安装的插件");
  });

  it("已安装插件超过 5 行：切入插件管理分区后限高生效（隐藏期测量无效不误清）", async () => {
    mockedInvoke.mockResolvedValue({
      marketplaces: [
        {
          name: "openai-bundled",
          path: "C:/x/bundled",
          plugins: Array.from({ length: 6 }, (_, i) => ({
            id: `p${i}`,
            name: `p${i}`,
            installed: true,
            enabled: true,
            version: "1.0.0",
            interface: { displayName: `插件${i}` },
          })),
        },
      ],
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    // 默认停在「个性化」分区，插件分区 v-show 隐藏、offsetHeight 恒为 0：
    // 隐藏期测量无效，列表不应写入 max-height（修复前此处会因 h=0 被清空且不再重算）
    const list = wrapper.find(".installed-plugin-list");
    expect(list.attributes("style") ?? "").not.toContain("max-height");

    const heightSpy = vi
      .spyOn(HTMLElement.prototype, "offsetHeight", "get")
      .mockReturnValue(30);
    try {
      // 切入插件管理分区（分区变为可见）触发重算：前 5 行 × 30px = 150px
      const navItem = wrapper
        .findAll(".settings-nav-item")
        .find((i) => i.text().includes("插件管理"));
      expect(navItem).toBeTruthy();
      await navItem!.trigger("click");
      await flushPromises();
      await nextTick();
      expect(list.attributes("style") ?? "").toContain("max-height: 150px");
    } finally {
      heightSpy.mockRestore();
    }
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
    // 默认折叠（断言市场内的列表，与已安装插件卡片的 .plugin-list 区分）
    expect(wrapper.find(".plugin-marketplace .plugin-list").exists()).toBe(
      false,
    );
    const head = wrapper.find(".plugin-marketplace-head");
    expect(head.classes()).toContain("collapsed");
    // 点击展开
    await head.trigger("click");
    expect(wrapper.find(".plugin-marketplace .plugin-list").exists()).toBe(
      true,
    );
    expect(head.classes()).not.toContain("collapsed");
    // 再点击折叠
    await head.trigger("click");
    expect(wrapper.find(".plugin-marketplace .plugin-list").exists()).toBe(
      false,
    );
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
    expect(wrapper.find(".plugin-marketplace .plugin-list").exists()).toBe(
      true,
    );
  });

  it("加载失败展示 marketplaceLoadErrors", async () => {
    mockedInvoke.mockResolvedValue({
      marketplaces: [],
      marketplaceLoadErrors: [{ name: "bad-repo", error: "git clone 失败" }],
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    expect(wrapper.find(".plugin-load-error").text()).toContain(
      "git clone 失败",
    );
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
    // 非 chrome 桥接插件不做进程预检
    expect(mockedInvoke).not.toHaveBeenCalledWith("browser_bridge_status");
    expect(mockedInvoke).not.toHaveBeenCalledWith("browser_bridge_stop");
  });

  /** chrome 插件安装用例的公共 mock：plugin/list 里只有 chrome 一项 */
  function mockChromePluginList(
    extra: (cmd: string, args?: any) => unknown | undefined,
  ) {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "plugin/list") {
        return {
          marketplaces: [
            {
              name: "openai-bundled",
              path: "C:/x/bundled",
              plugins: [
                {
                  id: "chrome@openai-bundled",
                  name: "chrome",
                  installed: false,
                  interface: { displayName: "ChatGPT 浏览器插件" },
                },
              ],
            },
          ],
        };
      }
      const handled = extra(cmd, args);
      return handled === undefined ? {} : handled;
    });
  }

  it("安装 chrome 插件时桥接进程运行：确认后先结束进程再安装", async () => {
    mockChromePluginList((cmd) => {
      if (cmd === "browser_bridge_status") {
        return { extensionHostRunning: true, nodeReplRunning: false };
      }
      if (cmd === "browser_bridge_stop") {
        return { stopped: ["extension-host.exe"], failed: [] };
      }
      if (cmd === "browser_bridge_repair") {
        return { status: "no-registration", latestAction: "none", message: "" };
      }
      return undefined;
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    await wrapper.find(".plugin-install-btn").trigger("click");
    await flushPromises();

    // 先弹预检确认，此时还没有安装
    expect(mockedInvoke).toHaveBeenCalledWith("browser_bridge_status");
    expect(store.confirm).toBeTruthy();
    expect(store.confirm!.title).toBe("浏览器桥接进程运行中");
    const installCalls = () =>
      mockedInvoke.mock.calls.filter(
        ([cmd, args]) =>
          cmd === "codex_rpc" &&
          (args as { method?: string } | undefined)?.method ===
            "plugin/install",
      );
    expect(installCalls()).toHaveLength(0);

    settleConfirm(true);
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("browser_bridge_stop");
    // 顺序：先结束进程，再安装
    const stopAt = mockedInvoke.mock.calls.findIndex(
      ([cmd]) => cmd === "browser_bridge_stop",
    );
    const installAt = mockedInvoke.mock.calls.findIndex(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "plugin/install",
    );
    expect(stopAt).toBeGreaterThanOrEqual(0);
    expect(installAt).toBeGreaterThan(stopAt);
    expect(store.toast).toContain("已安装 ChatGPT 浏览器插件");
  });

  it("安装 chrome 插件时用户取消预检：不结束进程也不安装", async () => {
    mockChromePluginList((cmd) => {
      if (cmd === "browser_bridge_status") {
        return { extensionHostRunning: false, nodeReplRunning: true };
      }
      return undefined;
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    await wrapper.find(".plugin-install-btn").trigger("click");
    await flushPromises();
    expect(store.confirm!.title).toBe("浏览器桥接进程运行中");

    settleConfirm(false);
    await flushPromises();
    expect(mockedInvoke).not.toHaveBeenCalledWith("browser_bridge_stop");
    expect(
      mockedInvoke.mock.calls.some(
        ([cmd, args]) =>
          cmd === "codex_rpc" &&
          (args as { method?: string } | undefined)?.method ===
            "plugin/install",
      ),
    ).toBe(false);
  });

  it("结束桥接进程失败时中止安装并给出中文指引", async () => {
    mockChromePluginList((cmd) => {
      if (cmd === "browser_bridge_status") {
        return { extensionHostRunning: true, nodeReplRunning: true };
      }
      if (cmd === "browser_bridge_stop") {
        return {
          stopped: [],
          failed: ["extension-host.exe（pid 1: 打开进程失败）"],
        };
      }
      return undefined;
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    await wrapper.find(".plugin-install-btn").trigger("click");
    await flushPromises();
    settleConfirm(true);
    await flushPromises();
    expect(store.toast).toContain("请完全退出 Chrome");
    expect(
      mockedInvoke.mock.calls.some(
        ([cmd, args]) =>
          cmd === "codex_rpc" &&
          (args as { method?: string } | undefined)?.method ===
            "plugin/install",
      ),
    ).toBe(false);
  });

  it("安装因缓存被占用失败（os error 5）时给中文指引", async () => {
    mockChromePluginList((cmd, args) => {
      if (cmd === "codex_rpc" && args?.method === "plugin/install") {
        throw new Error(
          "failed to install plugin: failed to back up plugin cache entry: 拒绝访问。 (os error 5)",
        );
      }
      return undefined;
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    await wrapper.find(".plugin-install-btn").trigger("click");
    await flushPromises();
    expect(store.toast).toContain("插件缓存文件被浏览器桥接进程占用");
    expect(store.toast).not.toContain("failed to install plugin");
  });

  it("安装 chrome 插件后调用 browser_bridge_repair 并提示桥接就绪", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "plugin/list") {
        return {
          marketplaces: [
            {
              name: "openai-bundled",
              path: "C:/x/bundled",
              plugins: [
                {
                  id: "chrome@openai-bundled",
                  name: "chrome",
                  installed: false,
                  interface: { displayName: "ChatGPT 浏览器插件" },
                },
              ],
            },
          ],
        };
      }
      if (cmd === "browser_bridge_repair") {
        return {
          status: "ok",
          latestAction: "created",
          latestTarget: "C:/x/chrome/26.730.61309",
          hostPathOk: true,
          clientPathOk: true,
          registryFixed: false,
          message: "已重建浏览器桥接链接",
        };
      }
      return {};
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    await wrapper.find(".plugin-install-btn").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("browser_bridge_repair");
    expect(store.toast).toContain("已安装 ChatGPT 浏览器插件");
    expect(store.toast).toContain("浏览器桥接已就绪");
  });

  it("安装 chrome 插件且自愈失败时提示原因", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "plugin/list") {
        return {
          marketplaces: [
            {
              name: "openai-bundled",
              path: "C:/x/bundled",
              plugins: [
                {
                  id: "chrome@openai-bundled",
                  name: "chrome",
                  installed: false,
                  interface: { displayName: "ChatGPT 浏览器插件" },
                },
              ],
            },
          ],
        };
      }
      if (cmd === "browser_bridge_repair") {
        return {
          status: "error",
          latestAction: "none",
          latestTarget: null,
          hostPathOk: false,
          clientPathOk: false,
          registryFixed: false,
          message: "设置 reparse point 失败",
        };
      }
      return {};
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-marketplace-head").trigger("click");
    await wrapper.find(".plugin-install-btn").trigger("click");
    await flushPromises();
    expect(store.toast).toContain("浏览器桥接自愈失败");
    expect(store.toast).toContain("设置 reparse point 失败");
  });

  it("安装非 chrome 插件不触发浏览器桥接自愈", async () => {
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
    expect(mockedInvoke).not.toHaveBeenCalledWith("browser_bridge_repair");
    expect(store.toast).toContain("已安装 PDF");
  });

  it("卸载 chrome 插件且桥接进程运行中时先弹预检确认", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "plugin/list") {
        return {
          marketplaces: [
            {
              name: "openai-bundled",
              path: "C:/x/bundled",
              plugins: [
                {
                  id: "chrome@openai-bundled",
                  name: "chrome",
                  installed: true,
                  interface: { displayName: "ChatGPT 浏览器插件" },
                },
              ],
            },
          ],
        };
      }
      if (cmd === "browser_bridge_status") {
        return { extensionHostRunning: true, nodeReplRunning: false };
      }
      return {};
    });
    wrapper = mount(SettingsView);
    await flushPromises();
    await wrapper.find(".plugin-uninstall-btn").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("browser_bridge_status");
    expect(store.confirm).toBeTruthy();
    expect(store.confirm!.title).toBe("浏览器桥接进程运行中");
    settleConfirm(true);
    await flushPromises();
    // 预检通过后仍有第二道卸载确认
    expect(store.confirm).toBeTruthy();
    expect(store.confirm!.title).toBe("卸载插件");
    settleConfirm(true);
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "plugin/uninstall",
      params: { pluginId: "chrome@openai-bundled" },
    });
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
    await wrapper.find(".plugin-market-add input").setValue("owner/repo");
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
    expect(
      wrapper.find(".plugin-install-btn").attributes("disabled"),
    ).toBeDefined();
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

  it("插件无任何图标时渲染品牌色首字母回退", async () => {
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

  it("图标加载失败后切换到品牌色首字母回退", async () => {
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
      if (cmd === "model_config_read")
        return Promise.resolve(sampleModelConfig);
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
      ".model-config-actions .model-config-save-btn",
      ".model-config-reload-btn",
      ".model-provider-actions .btn",
      "button.codex-pick-btn",
      "button.codex-clear-btn",
      "button.memory-delete-btn",
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
    // AGENTS / 模型目录标题链接为纯文字链接（无图标），仅验证文案完整
    for (const sel of [".model-config-title-link"]) {
      const buttons = wrapper.findAll(sel);
      expect(buttons.length, `${sel} 未找到按钮`).toBeGreaterThan(0);
      for (const btn of buttons) {
        expect(btn.text().trim().length).toBeGreaterThan(0);
        expect(btn.text().trim().endsWith("…")).toBe(false);
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
