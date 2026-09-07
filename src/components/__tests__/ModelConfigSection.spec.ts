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
} from "../../composables/useCodex";
import type { ModelProviderConfigState } from "../../composables/useCodex";

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
    expect(labels).toContain("personality（回复风格）");
    expect(labels).toContain("model_verbosity（输出详细程度）");
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
