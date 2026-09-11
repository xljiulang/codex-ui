import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return {
    ...mod,
    loadModelProviderConfig: vi.fn(),
    saveModelProviderConfig: vi.fn(async () => {}),
    setToast: vi.fn(),
    toastError: vi.fn((e: unknown) => String(e)),
  };
});

const mockSnapshotList = vi.hoisted(() => vi.fn());
const mockSnapshotApply = vi.hoisted(() => vi.fn());
vi.mock("../../composables/useModelSnapshots", () => ({
  listModelSnapshots: mockSnapshotList,
  createModelSnapshot: vi.fn(),
  applyModelSnapshot: mockSnapshotApply,
  deleteModelSnapshot: vi.fn(),
  openModelSnapshot: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import ModelConfigSection from "../settings/ModelConfigSection.vue";
import {
  loadModelProviderConfig,
  saveModelProviderConfig,
  setToast,
  toastError,
} from "../../composables/useCodex";
import type { ModelProviderConfigState } from "../../composables/useCodex";
import { tooltipDirective } from "../../directives/tooltip";
import { ICON_ARROW_CIRCLE_DOWN } from "../../lib/icons";

const mockedInvoke = vi.mocked(invoke);
const mockedLoad = vi.mocked(loadModelProviderConfig);
const mockedSave = vi.mocked(saveModelProviderConfig);

/** model_config_read 返回的最小骨架（本测试只关注 personality/model_verbosity 展示与保存） */
function modelConfigReadResult() {
  return {
    config_path: "C:\\Users\\t\\.codex\\config.toml",
    config_exists: true,
    config_content: "",
    model_catalog_json: "",
    model_catalog_path: "",
    model_catalog_exists: false,
    model_catalog: "",
    model: "gpt-5.2",
    model_reasoning_effort: "medium",
    model_provider: "",
    preferred_auth_method: "",
    forced_login_method: "",
    openai_api_key_present: false,
    providers: [],
  };
}

/** loadModelProviderConfig 返回的最小骨架 */
function providerConfigState(
  extra: Partial<ModelProviderConfigState> = {},
): ModelProviderConfigState {
  return {
    model: "gpt-5.2",
    model_reasoning_effort: "medium",
    model_reasoning_summary: "",
    personality: "",
    model_verbosity: "",
    model_provider: "",
    preferred_auth_method: "",
    forced_login_method: "",
    providers: [],
    raw: {},
    ...extra,
  };
}

async function mountSection() {
  const wrapper = mount(ModelConfigSection, {
    props: { active: true },
    // 注册真实 tooltip 指令：与 app 行为一致，并让 data-tip 可断言
    global: { directives: { tooltip: tooltipDirective } },
  });
  // 等待 onMounted 异步加载完成
  await vi.waitFor(() => expect(mockedLoad).toHaveBeenCalled());
  await vi.waitFor(() => expect(wrapper.find("#model-config-ui-personality").exists()).toBe(true));
  return wrapper;
}

beforeEach(() => {
  vi.clearAllMocks();
  mockedInvoke.mockReset();
  mockedInvoke.mockResolvedValue(modelConfigReadResult());
});

describe("ModelConfigSection 推理摘要 / 回复风格与输出详细程度", () => {
  it("渲染推理摘要、personality、model_verbosity 三个下拉并回显已配置值", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({
        model_reasoning_summary: "auto",
        personality: "pragmatic",
        model_verbosity: "low",
      }),
    );
    const wrapper = await mountSection();
    const rows = wrapper.findAll(".settings .setting-row");
    const labels = rows.map((r) => r.find("label").text());
    expect(labels).toContain("model_reasoning_effort（推理强度）");
    expect(labels).toContain("preferred_auth_method（优先认证方式）");
    expect(labels).toContain("forced_login_method（强制登录方式）");
    expect(labels).toContain("personality（回复风格）");
    expect(labels).toContain("model_reasoning_summary（推理摘要）");
    expect(labels).toContain("model_verbosity（输出详细程度）");
    // 顺序：effort → 推理摘要 → 优先认证 → 强制登录 → personality → verbosity
    const iEffort = labels.indexOf("model_reasoning_effort（推理强度）");
    const iAuth = labels.indexOf("preferred_auth_method（优先认证方式）");
    const iForced = labels.indexOf("forced_login_method（强制登录方式）");
    const iPersonality = labels.indexOf("personality（回复风格）");
    const iSummary = labels.indexOf("model_reasoning_summary（推理摘要）");
    const iVerbosity = labels.indexOf("model_verbosity（输出详细程度）");
    expect(iSummary).toBeGreaterThan(iEffort);
    expect(iAuth).toBeGreaterThan(iSummary);
    expect(iForced).toBeGreaterThan(iAuth);
    expect(iPersonality).toBeGreaterThan(iForced);
    expect(iVerbosity).toBeGreaterThan(iPersonality);
    // 推理强度与推理摘要相邻（两个一起改好操作）
    const uiFieldIds = wrapper
      .findAll("label[for^='model-config-ui-']")
      .map((label) => label.attributes("for"));
    expect(uiFieldIds[uiFieldIds.indexOf("model-config-ui-effort") + 1]).toBe(
      "model-config-ui-reasoning-summary",
    );
    // AppSelect 触发按钮显示选中项 label
    const personalitySelect = wrapper.find("#model-config-ui-personality");
    expect(personalitySelect.text()).toContain("pragmatic（务实简洁）");
    const summarySelect = wrapper.find("#model-config-ui-reasoning-summary");
    expect(summarySelect.text()).toContain("auto（自动）");
    const verbositySelect = wrapper.find("#model-config-ui-verbosity");
    expect(verbositySelect.text()).toContain("low（简洁）");
  });

  it("未配置时显示「默认（不写入）」", async () => {
    mockedLoad.mockResolvedValue(providerConfigState());
    const wrapper = await mountSection();
    expect(wrapper.find("#model-config-ui-reasoning-summary").text()).toContain(
      "默认（不写入）",
    );
    expect(wrapper.find("#model-config-ui-personality").text()).toContain("默认（不写入）");
    expect(wrapper.find("#model-config-ui-verbosity").text()).toContain("默认（不写入）");
  });

  it("保存时把当前选择传入 saveModelProviderConfig", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({
        model_reasoning_summary: "auto",
        personality: "pragmatic",
        model_verbosity: "low",
      }),
    );
    const wrapper = await mountSection();
    await wrapper.find(".model-config-save-btn").trigger("click");
    await vi.waitFor(() => expect(mockedSave).toHaveBeenCalled());
    const input = mockedSave.mock.calls[0]![0];
    expect(input.model_reasoning_summary).toBe("auto");
    expect(input.personality).toBe("pragmatic");
    expect(input.model_verbosity).toBe("low");
  });
});

describe("ModelConfigSection 生成模型目录", () => {
  const providerWithKey = {
    key: "deepseek",
    name: "DeepSeek",
    base_url: "https://api.deepseek.com/v1",
    env_key: "",
    experimental_bearer_token: "sk-test",
    wire_api: "responses",
  };
  const providerWithoutKey = {
    key: "other",
    name: "Other",
    base_url: "https://other.example.com/v1",
    env_key: "OTHER_API_KEY",
    experimental_bearer_token: "",
    wire_api: "responses",
  };

  /** 生成按钮绑定「当前选中的提供方」：默认选中 deepseek（同时有 base_url 与 API Key）。 */
  function activeProviderState(
    extra: Partial<ModelProviderConfigState> = {},
  ): ModelProviderConfigState {
    return providerConfigState({
      providers: [providerWithKey],
      model_provider: "deepseek",
      ...extra,
    });
  }

  function catalogResult() {
    return {
      catalog: JSON.stringify(
        {
          models: [
            {
              slug: "deepseek-chat",
              display_name: "Deepseek-Chat",
              priority: 1,
            },
            {
              slug: "deepseek-reasoner",
              display_name: "Deepseek-Reasoner",
              priority: 2,
            },
          ],
        },
        null,
        2,
      ),
      total: 3,
      ready: 2,
      incompatible: 0,
      unmatched: 1,
      models: [
        {
          id: "deepseek-chat",
          display_name: "Deepseek-Chat",
          status: "ready",
          selectable: true,
          sources: [
            {
              source: "models_dev",
              matched_id: "deepseek-chat",
              match_kind: "exact",
              score: 1,
            },
            {
              source: "openrouter",
              matched_id: "deepseek/deepseek-v3.2",
              match_kind: "fuzzy",
              score: 0.86,
            },
          ],
          warnings: [],
        },
        {
          id: "deepseek-reasoner",
          display_name: "Deepseek-Reasoner",
          status: "ready",
          selectable: true,
          sources: [
            {
              source: "official",
              matched_id: "deepseek-reasoner",
              match_kind: "exact",
              score: 1,
            },
          ],
          warnings: [],
        },
        {
          id: "unknown-model",
          display_name: "Unknown-Model",
          status: "unmatched",
          selectable: false,
          sources: [],
          warnings: ["未匹配到可用资料"],
        },
      ],
    };
  }

  it("生成按钮常驻模型目录区块右上角，提供方行内不再渲染", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({
        providers: [providerWithKey, providerWithoutKey],
        model_provider: "deepseek",
      }),
    );
    const wrapper = await mountSection();
    const rows = wrapper.findAll(".model-provider-row:not(.model-provider-none)");
    expect(rows[0].find(".model-catalog-generate").exists()).toBe(false);
    expect(rows[1].find(".model-catalog-generate").exists()).toBe(false);

    const generateButton = wrapper.find(
      ".model-catalog-head .model-catalog-generate",
    );
    expect(generateButton.exists()).toBe(true);
    expect(generateButton.find("svg").exists()).toBe(true);
    expect(
      generateButton.find("path").attributes("d"),
    ).toBe(ICON_ARROW_CIRCLE_DOWN);
    expect(generateButton.attributes("aria-label")).toBe("生成模型目录");
    expect(generateButton.attributes("disabled")).toBeUndefined();
    // tooltip 挂在包裹元素上（原生禁用按钮不派发鼠标事件，挂按钮上禁用态不会显示）
    const tip = wrapper.find(".model-catalog-head .model-catalog-generate-wrap");
    expect(tip.attributes("data-tip")).toBe("生成模型目录");
    expect(tip.attributes("data-tip")).not.toContain("API Key");

    // 标题本身即文件链接，与生成按钮同处标题行（默认 mock 未解析出路径 → 禁用）
    const titleLink = wrapper.find(".model-catalog-head .model-config-title-link");
    expect(titleLink.exists()).toBe(true);
    expect(titleLink.text()).toBe("model_catalog_json（模型目录）");
    expect(titleLink.attributes("disabled")).toBeDefined();
    expect(wrapper.find(".model-catalog-title-wrap").attributes("data-tip")).toBe(
      "正在读取目录路径…",
    );
  });

  it("模型目录区块位于模型提供方之后、模型标识之前", async () => {
    mockedLoad.mockResolvedValue(activeProviderState());
    const wrapper = await mountSection();
    const html = wrapper.html();
    const providersAt = html.indexOf("model-providers-list");
    const catalogAt = html.indexOf("model-catalog-block");
    const modelAt = html.indexOf("model-config-ui-model");
    expect(providersAt).toBeGreaterThan(-1);
    expect(catalogAt).toBeGreaterThan(providersAt);
    expect(modelAt).toBeGreaterThan(catalogAt);
  });

  it("模型目录标题即文件链接：文件存在时可点击，独立路径行已删除", async () => {
    mockedLoad.mockResolvedValue(activeProviderState());
    mockedInvoke.mockResolvedValue({
      ...modelConfigReadResult(),
      model_catalog_path: "C:\\Users\\t\\.codex\\models.json",
      model_catalog_exists: true,
      model_catalog: '{"models":[]}',
    });
    const wrapper = await mountSection();
    const link = wrapper.find(".model-catalog-head .model-config-title-link");
    expect(link.exists()).toBe(true);
    expect(link.text()).toBe("model_catalog_json（模型目录）");
    expect(link.attributes("disabled")).toBeUndefined();
    expect(wrapper.find(".model-catalog-title-wrap").attributes("data-tip")).toBe(
      "在编辑器中打开 C:\\Users\\t\\.codex\\models.json",
    );
    // 路径行与生成按钮同处标题行，不再有独立路径元素
    expect(
      wrapper.find(".model-catalog-block .model-config-path").exists(),
    ).toBe(false);
    expect(
      wrapper.find(".model-catalog-block .model-config-path-link").exists(),
    ).toBe(false);
    expect(
      wrapper.find(".model-catalog-head .model-catalog-generate").exists(),
    ).toBe(true);
  });

  it("模型目录文件缺失时标题链接禁用，tooltip 提示保存时将新建", async () => {
    mockedLoad.mockResolvedValue(activeProviderState());
    mockedInvoke.mockResolvedValue({
      ...modelConfigReadResult(),
      model_catalog_path: "C:\\Users\\t\\.codex\\models.json",
      model_catalog_exists: false,
      model_catalog: "",
    });
    const wrapper = await mountSection();
    const link = wrapper.find(".model-catalog-head .model-config-title-link");
    expect(link.attributes("disabled")).toBeDefined();
    const tip = wrapper
      .find(".model-catalog-title-wrap")
      .attributes("data-tip") as string;
    expect(tip).toContain("models.json");
    expect(tip).toContain("文件不存在，保存时将新建");
  });

  it("未选择提供方时按钮禁用并提示，点击不发起生成", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({ providers: [providerWithKey], model_provider: "" }),
    );
    const wrapper = await mountSection();
    const button = wrapper.find(".model-catalog-generate");
    expect(button.attributes("disabled")).toBeDefined();
    expect(
      wrapper.find(".model-catalog-generate-wrap").attributes("data-tip"),
    ).toBe("请先选择一个模型提供方");

    await button.trigger("click");
    expect(
      mockedInvoke.mock.calls.some(
        ([cmd]) => cmd === "model_catalog_generate_from_provider",
      ),
    ).toBe(false);
    expect(wrapper.find(".model-catalog-picker").exists()).toBe(false);
  });

  it("选中缺 API Key 的提供方时按钮禁用并提示原因", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({
        providers: [providerWithKey, providerWithoutKey],
        model_provider: "other",
      }),
    );
    const wrapper = await mountSection();
    const button = wrapper.find(".model-catalog-generate");
    expect(button.attributes("disabled")).toBeDefined();
    expect(
      wrapper.find(".model-catalog-generate-wrap").attributes("data-tip"),
    ).toBe("当前提供方缺少 base_url 或 API Key");

    await button.trigger("click");
    expect(
      mockedInvoke.mock.calls.some(
        ([cmd]) => cmd === "model_catalog_generate_from_provider",
      ),
    ).toBe(false);
  });

  it("切换提供方选择时按钮可用性跟随变化", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({
        providers: [providerWithKey, providerWithoutKey],
        model_provider: "other",
      }),
    );
    const wrapper = await mountSection();
    const button = wrapper.find(".model-catalog-generate");
    expect(button.attributes("disabled")).toBeDefined();

    // 0 = 不使用提供者，1 = deepseek（有 base_url + API Key），2 = other
    const radios = wrapper.findAll('input[name="model-provider-active"]');
    await radios[1].setValue();
    expect(button.attributes("disabled")).toBeUndefined();
    expect(
      wrapper.find(".model-catalog-generate-wrap").attributes("data-tip"),
    ).toBe("生成模型目录");

    await radios[0].setValue();
    expect(button.attributes("disabled")).toBeDefined();
  });

  it("wire_api 仅支持 responses：历史 chat 行内报错并回填 responses", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({
        providers: [
          {
            key: "legacy",
            name: "Legacy",
            base_url: "https://legacy.example.com/v1",
            env_key: "",
            experimental_bearer_token: "sk-legacy",
            wire_api: "chat",
          },
        ],
      }),
    );
    const wrapper = await mountSection();

    const row = wrapper.find(".model-provider-row:not(.model-provider-none)");
    expect(row.classes()).toContain("model-provider-row-error");
    expect(row.text()).toContain("wire_api 仅支持 responses（当前 chat）");

    // 打开编辑弹窗：唯一合法值 responses 已回填（保存一次即修正历史 chat）
    await row.find(".provider-row-edit").trigger("click");
    await flushPromises();
    expect(wrapper.find(".modal .app-select").text()).toBe("responses");
    // wire_api 选项只剩 responses（弹层 Teleport 到 body）
    await wrapper.find(".modal .app-select").trigger("click");
    await flushPromises();
    expect(
      [
        ...document.body.querySelectorAll(
          ".app-select-menu .app-select-option",
        ),
      ].map((el) => el.textContent?.trim()),
    ).toEqual(["responses"]);
    wrapper.unmount();
  });

  it("点击后打开选择弹窗且确认前不修改编辑框", async () => {
    mockedLoad.mockResolvedValue(
      activeProviderState(),
    );
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") {
        return Promise.resolve({
          ...modelConfigReadResult(),
          model_catalog: "old-catalog",
        });
      }
      if (cmd === "model_catalog_generate_from_provider") {
        return Promise.resolve(catalogResult());
      }
      return Promise.resolve(undefined);
    });
    const wrapper = await mountSection();
    await wrapper.find(".model-catalog-generate").trigger("click");
    await vi.waitFor(() =>
      expect(wrapper.find(".model-catalog-picker").exists()).toBe(true),
    );
    expect(mockedInvoke).toHaveBeenCalledWith(
      "model_catalog_generate_from_provider",
      {
        baseUrl: "https://api.deepseek.com/v1",
        apiKey: "sk-test",
      },
    );
    expect(
      (wrapper.find(".model-config-textarea").element as HTMLTextAreaElement)
        .value,
    ).toBe("old-catalog");
    const checkboxes = wrapper.findAll(
      ".model-catalog-picker-row input[type='checkbox']",
    );
    expect(checkboxes).toHaveLength(3);
    expect((checkboxes[0].element as HTMLInputElement).checked).toBe(false);
    expect((checkboxes[1].element as HTMLInputElement).checked).toBe(false);
    expect((checkboxes[2].element as HTMLInputElement).checked).toBe(false);
    expect((checkboxes[2].element as HTMLInputElement).disabled).toBe(true);
    const selectAll = wrapper.find(
      ".model-catalog-picker-select-all input[type='checkbox']",
    );
    expect((selectAll.element as HTMLInputElement).checked).toBe(false);
    expect((selectAll.element as HTMLInputElement).indeterminate).toBe(false);
    const confirm = wrapper.find(".model-catalog-picker-confirm");
    expect(confirm.text()).toContain("生成 0 个条目");
    expect(confirm.attributes("disabled")).toBeDefined();
    expect(confirm.classes()).not.toContain("primary");
  });

  it("弹窗为每个命中资料展示独立来源徽章", async () => {
    mockedLoad.mockResolvedValue(
      activeProviderState(),
    );
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") {
        return Promise.resolve(modelConfigReadResult());
      }
      if (cmd === "model_catalog_generate_from_provider") {
        return Promise.resolve(catalogResult());
      }
      return Promise.resolve(undefined);
    });
    const wrapper = await mountSection();
    await wrapper.find(".model-catalog-generate").trigger("click");
    await vi.waitFor(() =>
      expect(wrapper.find(".model-catalog-picker").exists()).toBe(true),
    );

    // 同一候选的每个来源各一枚徽章，不再拼成一条
    const badgeGroups = wrapper.findAll(".model-catalog-picker-sources");
    expect(badgeGroups).toHaveLength(2);
    expect(
      badgeGroups.flatMap((group) =>
        group
          .findAll(".model-catalog-picker-source")
          .map((badge) => badge.text()),
      ),
    ).toEqual([
      "models.dev",
      "OpenRouter · 继承 deepseek/deepseek-v3.2",
      "官方条目",
    ]);
    // 未命中的模型只显示跳过提示，不给来源标记
    expect(wrapper.findAll(".model-catalog-picker-status")).toHaveLength(1);
  });

  it("全部不可用时仍打开弹窗并展示不兼容与未匹配原因", async () => {
    mockedLoad.mockResolvedValue(
      activeProviderState(),
    );
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") return Promise.resolve(modelConfigReadResult());
      if (cmd === "model_catalog_generate_from_provider") {
        return Promise.resolve({
          catalog: '{"models":[]}',
          total: 2,
          ready: 0,
          incompatible: 1,
          unmatched: 1,
          models: [
            {
              id: "legacy-model",
              display_name: "Legacy-Model",
              status: "incompatible",
              selectable: false,
              sources: [
                {
                  source: "models_dev",
                  matched_id: "legacy-model",
                  match_kind: "exact",
                  score: 1,
                },
              ],
              warnings: ["上游明确标记 tool_call=false，不兼容 Codex 工具调用"],
            },
            {
              id: "unknown-model",
              display_name: "Unknown-Model",
              status: "unmatched",
              selectable: false,
              sources: [],
              warnings: ["未匹配到可用资料"],
            },
          ],
        });
      }
      return Promise.resolve(undefined);
    });
    const wrapper = await mountSection();
    await wrapper.find(".model-catalog-generate").trigger("click");
    await vi.waitFor(() =>
      expect(wrapper.find(".model-catalog-picker").exists()).toBe(true),
    );

    expect(wrapper.find(".model-catalog-picker-meta").text()).toContain(
      "可生成 0 个，不兼容 1 个，未匹配 1 个",
    );
    expect(wrapper.findAll(".model-catalog-picker-row.is-incompatible")).toHaveLength(1);
    expect(wrapper.findAll(".model-catalog-picker-row.is-unmatched")).toHaveLength(1);
    expect(wrapper.find(".model-catalog-picker-list").text()).toContain(
      "tool_call=false",
    );
    expect(
      wrapper
        .findAll(".model-catalog-picker-row input[type='checkbox']")
        .every((item) => (item.element as HTMLInputElement).disabled),
    ).toBe(true);
  });

  it("点击模型文字不切换复选框，点击复选框仍可正常选择", async () => {
    mockedLoad.mockResolvedValue(
      activeProviderState(),
    );
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") return Promise.resolve(modelConfigReadResult());
      if (cmd === "model_catalog_generate_from_provider") {
        return Promise.resolve(catalogResult());
      }
      return Promise.resolve(undefined);
    });
    const wrapper = await mountSection();
    await wrapper.find(".model-catalog-generate").trigger("click");
    await vi.waitFor(() =>
      expect(wrapper.find(".model-catalog-picker").exists()).toBe(true),
    );

    await wrapper.find(".model-catalog-picker-id").trigger("click");
    const firstCheckbox = wrapper.find(
      ".model-catalog-picker-row input[type='checkbox']",
    );
    expect((firstCheckbox.element as HTMLInputElement).checked).toBe(false);
    expect(wrapper.find(".model-catalog-picker-confirm").attributes("disabled"))
      .toBeDefined();

    await firstCheckbox.setValue(true);
    expect((firstCheckbox.element as HTMLInputElement).checked).toBe(true);
    expect(wrapper.find(".model-catalog-picker-confirm").text()).toContain(
      "生成 1 个条目",
    );
  });

  it("三态全选复选框覆盖全部、部分和未选状态", async () => {
    mockedLoad.mockResolvedValue(
      activeProviderState(),
    );
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") return Promise.resolve(modelConfigReadResult());
      if (cmd === "model_catalog_generate_from_provider") {
        return Promise.resolve(catalogResult());
      }
      return Promise.resolve(undefined);
    });
    const wrapper = await mountSection();
    await wrapper.find(".model-catalog-generate").trigger("click");
    await vi.waitFor(() =>
      expect(wrapper.find(".model-catalog-picker").exists()).toBe(true),
    );

    expect(wrapper.find(".model-catalog-picker-toolbar .btn").exists()).toBe(
      false,
    );
    const selectAll = wrapper.find(
      ".model-catalog-picker-select-all input[type='checkbox']",
    );
    expect((selectAll.element as HTMLInputElement).checked).toBe(false);
    expect((selectAll.element as HTMLInputElement).indeterminate).toBe(false);

    await selectAll.trigger("change");
    expect((selectAll.element as HTMLInputElement).checked).toBe(true);
    expect((selectAll.element as HTMLInputElement).indeterminate).toBe(false);
    expect(wrapper.find(".model-catalog-picker-count").text()).toContain(
      "已选 2",
    );

    await wrapper
      .findAll(".model-catalog-picker-row input[type='checkbox']")[0]
      .setValue(false);
    expect((selectAll.element as HTMLInputElement).checked).toBe(false);
    expect((selectAll.element as HTMLInputElement).indeterminate).toBe(true);

    await selectAll.trigger("change");
    expect((selectAll.element as HTMLInputElement).checked).toBe(true);

    await selectAll.trigger("change");
    expect((selectAll.element as HTMLInputElement).checked).toBe(false);
    expect((selectAll.element as HTMLInputElement).indeterminate).toBe(false);
    expect(
      wrapper.find(".model-catalog-picker-confirm").attributes("disabled"),
    ).toBeDefined();
  });

  it("搜索时三态全选只操作当前结果并保留隐藏项", async () => {
    mockedLoad.mockResolvedValue(
      activeProviderState(),
    );
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") return Promise.resolve(modelConfigReadResult());
      if (cmd === "model_catalog_generate_from_provider") {
        return Promise.resolve(catalogResult());
      }
      return Promise.resolve(undefined);
    });
    const wrapper = await mountSection();
    await wrapper.find(".model-catalog-generate").trigger("click");
    await vi.waitFor(() =>
      expect(wrapper.find(".model-catalog-picker").exists()).toBe(true),
    );

    const search = wrapper.find(".model-catalog-picker-search");
    const selectAll = wrapper.find(
      ".model-catalog-picker-select-all input[type='checkbox']",
    );
    await selectAll.trigger("change");
    await search.setValue("reasoner");
    expect(wrapper.findAll(".model-catalog-picker-row")).toHaveLength(1);
    expect((selectAll.element as HTMLInputElement).checked).toBe(true);

    await selectAll.trigger("change");
    expect(
      (
        wrapper.find(".model-catalog-picker-row input[type='checkbox']")
          .element as HTMLInputElement
      ).checked,
    ).toBe(false);
    expect(wrapper.find(".model-catalog-picker-count").text()).toContain(
      "已选 1",
    );

    await search.setValue("");
    expect((selectAll.element as HTMLInputElement).indeterminate).toBe(true);
    const rows = wrapper.findAll(".model-catalog-picker-row input[type='checkbox']");
    expect((rows[0].element as HTMLInputElement).checked).toBe(true);
    expect((rows[1].element as HTMLInputElement).checked).toBe(false);

    await selectAll.trigger("change");
    expect((selectAll.element as HTMLInputElement).checked).toBe(true);

    await search.setValue("unknown");
    expect((selectAll.element as HTMLInputElement).disabled).toBe(true);
  });

  it("确认后只写入选中模型、重排 priority 并提示数量", async () => {
    mockedLoad.mockResolvedValue(
      activeProviderState(),
    );
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") {
        return Promise.resolve({
          ...modelConfigReadResult(),
          model_catalog: "old-catalog",
        });
      }
      if (cmd === "model_catalog_generate_from_provider") {
        return Promise.resolve(catalogResult());
      }
      return Promise.resolve(undefined);
    });
    const wrapper = await mountSection();
    await wrapper.find(".model-catalog-generate").trigger("click");
    await vi.waitFor(() =>
      expect(wrapper.find(".model-catalog-picker").exists()).toBe(true),
    );

    await wrapper
      .findAll(".model-catalog-picker-row input[type='checkbox']")[1]
      .setValue(true);
    await wrapper.find(".model-catalog-picker-confirm").trigger("click");
    await vi.waitFor(() =>
      expect(wrapper.find(".model-catalog-picker").exists()).toBe(false),
    );

    const catalog = JSON.parse(
      (wrapper.find(".model-config-textarea").element as HTMLTextAreaElement)
        .value,
    ) as { models: Array<{ slug: string; priority: number }> };
    expect(catalog.models).toEqual([
      { slug: "deepseek-reasoner", display_name: "Deepseek-Reasoner", priority: 1 },
    ]);
    expect(setToast).toHaveBeenCalledWith(
      "已生成 1 个模型条目，跳过 1 个不可用模型，保存并重启 codex-ui 后生效",
    );
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "model_catalog_save",
      expect.anything(),
    );
  });

  it("取消选择不修改编辑框", async () => {
    mockedLoad.mockResolvedValue(
      activeProviderState(),
    );
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") {
        return Promise.resolve({
          ...modelConfigReadResult(),
          model_catalog: "old-catalog",
        });
      }
      if (cmd === "model_catalog_generate_from_provider") {
        return Promise.resolve(catalogResult());
      }
      return Promise.resolve(undefined);
    });
    const wrapper = await mountSection();
    await wrapper.find(".model-catalog-generate").trigger("click");
    await vi.waitFor(() =>
      expect(wrapper.find(".model-catalog-picker").exists()).toBe(true),
    );
    await wrapper.find(".modal-foot .btn").trigger("click");
    expect(wrapper.find(".model-catalog-picker").exists()).toBe(false);
    expect(
      (wrapper.find(".model-config-textarea").element as HTMLTextAreaElement)
        .value,
    ).toBe("old-catalog");
  });

  it("生成失败时不打开弹窗、不修改编辑框并提示错误", async () => {
    mockedLoad.mockResolvedValue(
      activeProviderState(),
    );
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") {
        return Promise.resolve({
          ...modelConfigReadResult(),
          model_catalog: "old-catalog",
        });
      }
      if (cmd === "model_catalog_generate_from_provider") {
        return Promise.reject(new Error("请求失败"));
      }
      return Promise.resolve(undefined);
    });
    const wrapper = await mountSection();
    await wrapper.find(".model-catalog-generate").trigger("click");
    await vi.waitFor(() => expect(toastError).toHaveBeenCalled());
    expect(wrapper.find(".model-catalog-picker").exists()).toBe(false);
    expect(
      (wrapper.find(".model-config-textarea").element as HTMLTextAreaElement)
        .value,
    ).toBe("old-catalog");
  });

  it("生成期间按钮禁用", async () => {
    mockedLoad.mockResolvedValue(
      activeProviderState(),
    );
    let resolveGenerate!: (value: unknown) => void;
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") return Promise.resolve(modelConfigReadResult());
      if (cmd === "model_catalog_generate_from_provider") {
        return new Promise((resolve) => {
          resolveGenerate = resolve;
        });
      }
      return Promise.resolve(undefined);
    });
    const wrapper = await mountSection();
    const button = wrapper.find(".model-catalog-generate");
    await button.trigger("click");
    await vi.waitFor(() => expect(button.attributes("disabled")).toBeDefined());
    resolveGenerate(catalogResult());
    await vi.waitFor(() => expect(button.attributes("disabled")).toBeUndefined());
    expect(wrapper.find(".model-catalog-picker").exists()).toBe(true);
  });

  it("校验失败条目禁用勾选，确定后只回填选中项且不保存", async () => {
    mockedLoad.mockResolvedValue(activeProviderState());
    const result = catalogResult();
    const parsed = JSON.parse(result.catalog);
    parsed.models[0].context_window = 128000;
    result.catalog = JSON.stringify(parsed);
    const response = {
      ...result,
      invalid: 1,
      total: 4,
      models: [
        result.models[0],
        ...result.models.slice(1),
        { id: "broken", display_name: "Broken", status: "invalid", selectable: false,
          sources: [], warnings: ["上下文参数无效"] },
      ],
    };
    mockedInvoke.mockImplementation(async (cmd) => {
      if (cmd === "model_config_read") return modelConfigReadResult();
      if (cmd === "model_catalog_generate_from_provider") return response;
    });
    const wrapper = await mountSection();
    await wrapper.find(".model-catalog-generate").trigger("click");
    await flushPromises();
    expect(wrapper.find(".model-catalog-picker").text()).toContain("校验失败 1 个");
    const checkbox = wrapper.find(".model-catalog-picker-row input");
    expect((checkbox.element as HTMLInputElement).checked).toBe(false);
    // 弹窗不再提供参数来源入口
    expect(wrapper.find(".model-catalog-provenance-toggle").exists()).toBe(false);
    expect(wrapper.find(".model-catalog-provenance").exists()).toBe(false);
    expect(wrapper.find(".is-invalid input").attributes("disabled")).toBeDefined();
    await checkbox.setValue(true);
    await wrapper.find(".model-catalog-picker-confirm").trigger("click");
    expect(wrapper.find(".model-catalog-picker").exists()).toBe(false);
    const catalog = JSON.parse((wrapper.find(".model-config-textarea").element as HTMLTextAreaElement).value);
    expect(catalog.models).toHaveLength(1);
    expect(catalog.models[0]).toMatchObject({ slug: "deepseek-chat", context_window: 128000, priority: 1 });
    expect(mockedInvoke.mock.calls.filter(([cmd]) => cmd === "model_catalog_generate_from_provider")).toHaveLength(1);
    expect(mockedInvoke.mock.calls.some(([cmd]) => cmd === "model_catalog_save")).toBe(false);
    expect(mockedSave).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("全部校验失败时保留原目录且不能确认", async () => {
    mockedLoad.mockResolvedValue(activeProviderState());
    mockedInvoke.mockImplementation(async (cmd) => {
      if (cmd === "model_config_read") return { ...modelConfigReadResult(), model_catalog: "old-catalog" };
      if (cmd === "model_catalog_generate_from_provider") return {
        catalog: '{"models":[]}', total: 1, ready: 0, unmatched: 0, incompatible: 0, invalid: 1,
        models: [{ id: "broken", display_name: "Broken", status: "invalid", selectable: false,
          sources: [], warnings: ["上下文参数无效"] }],
      };
    });
    const wrapper = await mountSection();
    await wrapper.find(".model-catalog-generate").trigger("click");
    await flushPromises();
    expect(wrapper.find(".model-catalog-picker-confirm").attributes("disabled")).toBeDefined();
    expect(wrapper.find(".model-catalog-picker").text()).toContain("上下文参数无效");
    expect((wrapper.find(".model-config-textarea").element as HTMLTextAreaElement).value).toBe("old-catalog");
    wrapper.unmount();
  });

  it("组件卸载后忽略迟到的生成成功或失败", async () => {
    for (const fail of [false, true]) {
      vi.mocked(setToast).mockClear();
      mockedLoad.mockResolvedValue(activeProviderState());
      let finish!: () => void;
      mockedInvoke.mockImplementation((cmd) => {
        if (cmd === "model_config_read") return Promise.resolve(modelConfigReadResult());
        if (cmd === "model_catalog_generate_from_provider") {
          return new Promise((resolve, reject) => {
            finish = () => fail ? reject(new Error("迟到错误")) : resolve(catalogResult());
          });
        }
        return Promise.resolve(undefined);
      });
      const wrapper = await mountSection();
      await wrapper.find(".model-catalog-generate").trigger("click");
      wrapper.unmount();
      finish();
      await flushPromises();
      expect(setToast).not.toHaveBeenCalled();
    }
  });
});

describe("ModelConfigSection 模型快照联动", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue(modelConfigReadResult());
    mockedLoad.mockResolvedValue(providerConfigState());
    mockSnapshotList.mockReset().mockResolvedValue(["dev"]);
    mockSnapshotApply.mockReset().mockResolvedValue(undefined);
  });

  it("还原模型快照后自动重读一次模型配置", async () => {
    const wrapper = await mountSection();
    const readCount = () =>
      mockedInvoke.mock.calls.filter(
        ([cmd]) => cmd === "model_config_read",
      ).length;
    expect(readCount()).toBe(1);

    await wrapper
      .find('button[aria-label="还原模型快照dev"]')
      .trigger("click");
    await flushPromises();

    expect(mockSnapshotApply).toHaveBeenCalledWith("dev");
    expect(readCount()).toBe(2);
    wrapper.unmount();
  });
});
