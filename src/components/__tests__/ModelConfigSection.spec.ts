import { beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";

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

import { invoke } from "@tauri-apps/api/core";
import ModelConfigSection from "../settings/ModelConfigSection.vue";
import {
  loadModelProviderConfig,
  saveModelProviderConfig,
  setToast,
  toastError,
} from "../../composables/useCodex";
import type { ModelProviderConfigState } from "../../composables/useCodex";
import { ICON_SWAP_HORIZ } from "../../lib/icons";

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
  const wrapper = mount(ModelConfigSection, { props: { active: true } });
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

describe("ModelConfigSection 回复风格与输出详细程度", () => {
  it("渲染 personality / model_verbosity 两个下拉并回显已配置值", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({ personality: "pragmatic", model_verbosity: "low" }),
    );
    const wrapper = await mountSection();
    const rows = wrapper.findAll(".settings .setting-row");
    const labels = rows.map((r) => r.find("label").text());
    expect(labels).toContain("model_reasoning_effort（推理强度）");
    expect(labels).toContain("preferred_auth_method（优先认证方式）");
    expect(labels).toContain("forced_login_method（强制登录方式）");
    expect(labels).toContain("personality（回复风格）");
    expect(labels).toContain("model_verbosity（输出详细程度）");
    // 顺序：effort → 优先认证 → 强制登录 → personality → verbosity
    const iEffort = labels.indexOf("model_reasoning_effort（推理强度）");
    const iAuth = labels.indexOf("preferred_auth_method（优先认证方式）");
    const iForced = labels.indexOf("forced_login_method（强制登录方式）");
    const iPersonality = labels.indexOf("personality（回复风格）");
    const iVerbosity = labels.indexOf("model_verbosity（输出详细程度）");
    expect(iAuth).toBeGreaterThan(iEffort);
    expect(iForced).toBeGreaterThan(iAuth);
    expect(iPersonality).toBeGreaterThan(iForced);
    expect(iVerbosity).toBeGreaterThan(iPersonality);
    // AppSelect 触发按钮显示选中项 label
    const personalitySelect = wrapper.find("#model-config-ui-personality");
    expect(personalitySelect.text()).toContain("pragmatic（务实简洁）");
    const verbositySelect = wrapper.find("#model-config-ui-verbosity");
    expect(verbositySelect.text()).toContain("low（简洁）");
  });

  it("未配置时显示「默认（不写入）」", async () => {
    mockedLoad.mockResolvedValue(providerConfigState());
    const wrapper = await mountSection();
    expect(wrapper.find("#model-config-ui-personality").text()).toContain("默认（不写入）");
    expect(wrapper.find("#model-config-ui-verbosity").text()).toContain("默认（不写入）");
  });

  it("保存时把当前选择传入 saveModelProviderConfig", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({ personality: "pragmatic", model_verbosity: "low" }),
    );
    const wrapper = await mountSection();
    await wrapper.find(".model-config-save-btn").trigger("click");
    await vi.waitFor(() => expect(mockedSave).toHaveBeenCalled());
    const input = mockedSave.mock.calls[0]![0];
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
    wire_api: "chat",
  };

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
      matched: 2,
      skipped: 1,
      models: [
        {
          id: "deepseek-chat",
          display_name: "Deepseek-Chat",
          matched: true,
          official: false,
          models_dev: true,
          openrouter: true,
        },
        {
          id: "deepseek-reasoner",
          display_name: "Deepseek-Reasoner",
          matched: true,
          official: true,
          models_dev: false,
          openrouter: false,
        },
        {
          id: "unknown-model",
          display_name: "Unknown-Model",
          matched: false,
          official: false,
          models_dev: false,
          openrouter: false,
        },
      ],
    };
  }

  it("仅对有 base_url 和 API Key 的提供方显示生成按钮", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({ providers: [providerWithKey, providerWithoutKey] }),
    );
    const wrapper = await mountSection();
    const rows = wrapper.findAll(".model-provider-row:not(.model-provider-none)");
    expect(rows[0].find(".provider-row-generate").exists()).toBe(true);
    expect(rows[0].find(".provider-row-generate svg").exists()).toBe(true);
    expect(
      rows[0].find(".provider-row-generate path").attributes("d"),
    ).toBe(ICON_SWAP_HORIZ);
    expect(rows[1].find(".provider-row-generate").exists()).toBe(false);
  });

  it("点击后打开选择弹窗且确认前不修改编辑框", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({ providers: [providerWithKey] }),
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
    await wrapper.find(".provider-row-generate").trigger("click");
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

  it("弹窗展示每个候选命中资料的来源标记", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({ providers: [providerWithKey] }),
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
    await wrapper.find(".provider-row-generate").trigger("click");
    await vi.waitFor(() =>
      expect(wrapper.find(".model-catalog-picker").exists()).toBe(true),
    );

    const labels = wrapper
      .findAll(".model-catalog-picker-source")
      .map((label) => label.text());
    expect(labels).toEqual(["models.dev + OpenRouter", "官方条目"]);
    // 未命中的模型只显示跳过提示，不给来源标记
    expect(wrapper.findAll(".model-catalog-picker-status")).toHaveLength(1);
  });

  it("点击模型文字不切换复选框，点击复选框仍可正常选择", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({ providers: [providerWithKey] }),
    );
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") return Promise.resolve(modelConfigReadResult());
      if (cmd === "model_catalog_generate_from_provider") {
        return Promise.resolve(catalogResult());
      }
      return Promise.resolve(undefined);
    });
    const wrapper = await mountSection();
    await wrapper.find(".provider-row-generate").trigger("click");
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
      providerConfigState({ providers: [providerWithKey] }),
    );
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") return Promise.resolve(modelConfigReadResult());
      if (cmd === "model_catalog_generate_from_provider") {
        return Promise.resolve(catalogResult());
      }
      return Promise.resolve(undefined);
    });
    const wrapper = await mountSection();
    await wrapper.find(".provider-row-generate").trigger("click");
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
      providerConfigState({ providers: [providerWithKey] }),
    );
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === "model_config_read") return Promise.resolve(modelConfigReadResult());
      if (cmd === "model_catalog_generate_from_provider") {
        return Promise.resolve(catalogResult());
      }
      return Promise.resolve(undefined);
    });
    const wrapper = await mountSection();
    await wrapper.find(".provider-row-generate").trigger("click");
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
      providerConfigState({ providers: [providerWithKey] }),
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
    await wrapper.find(".provider-row-generate").trigger("click");
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
      "已生成 1 个模型条目，跳过 1 个未匹配模型，保存并重启 codex-ui 后生效",
    );
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "model_catalog_save",
      expect.anything(),
    );
  });

  it("取消选择不修改编辑框", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({ providers: [providerWithKey] }),
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
    await wrapper.find(".provider-row-generate").trigger("click");
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
      providerConfigState({ providers: [providerWithKey] }),
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
    await wrapper.find(".provider-row-generate").trigger("click");
    await vi.waitFor(() => expect(toastError).toHaveBeenCalled());
    expect(wrapper.find(".model-catalog-picker").exists()).toBe(false);
    expect(
      (wrapper.find(".model-config-textarea").element as HTMLTextAreaElement)
        .value,
    ).toBe("old-catalog");
  });

  it("生成期间按钮禁用", async () => {
    mockedLoad.mockResolvedValue(
      providerConfigState({ providers: [providerWithKey] }),
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
    const button = wrapper.find(".provider-row-generate");
    await button.trigger("click");
    await vi.waitFor(() => expect(button.attributes("disabled")).toBeDefined());
    resolveGenerate(catalogResult());
    await vi.waitFor(() => expect(button.attributes("disabled")).toBeUndefined());
    expect(wrapper.find(".model-catalog-picker").exists()).toBe(true);
  });
});
