<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  loadModelProviderConfig,
  saveModelProviderConfig,
  setToast,
  toastError,
} from "../../composables/useCodex";
import type {
  ModelProviderConfigState,
} from "../../composables/useCodex";
import { openPathInAppOrReveal } from "../../composables/usePathOpen";
import { openDocsUrl } from "../../lib/links";
import { filterCatalogByModelIds } from "../../lib/modelCatalog";
import {
  ICON_ARROW_CIRCLE_RIGHT,
  ICON_DELETE,
  ICON_EDIT,
  ICON_LINK,
  ICON_PLUS,
  ICON_REFRESH,
  ICON_SAVE,
} from "../../lib/icons";
import type {
  ModelCatalogGenerateResult,
  ModelCatalogModelOption,
  ModelConfigUiEdit,
  ModelConfigState,
  ModelProviderInfo,
} from "../../lib/types";
import AppSelect, { type AppSelectOption } from "../AppSelect.vue";
import ModelConfigModelPicker from "../ModelConfigModelPicker.vue";
import ModalDialog from "../ModalDialog.vue";
import ModelSnapshotCard from "./ModelSnapshotCard.vue";

defineProps<{ active: boolean }>();

/** DeepSeek Codex 接入文档（模型提供方配置参考，浏览器打开） */
const DEEPSEEK_CODEX_DOCS_URL =
  "https://api-docs.deepseek.com/zh-cn/quick_start/agent_integrations/codex/";

/** GLM（智谱）Codex 接入文档（浏览器打开） */
const GLM_CODEX_DOCS_URL = "https://docs.bigmodel.cn/cn/coding-plan/tool/codex";

/** config / model_catalog_json 卡片状态（字段与 Rust 端 model_config_read 返回一致） */
const modelConfig = reactive({
  loading: false,
  saving: false,
  config_path: "",
  config_exists: false,
  config_content: "",
  model_catalog_path: "",
  model_catalog_exists: false,
  model_catalog: "",
  model: "",
  model_reasoning_effort: "",
  personality: "",
  model_verbosity: "",
  model_provider: "",
  preferred_auth_method: "",
  forced_login_method: "",
  openai_api_key_present: false,
  providers: [] as ModelProviderInfo[],
});

/** app-server 协议 `model_reasoning_effort`（codex-cli ReasoningEffort）支持的档位。
 *  空串代表「默认（不写入）」，由模板单独渲染；新增档位时同步此数组。 */
const REASONING_EFFORT_VALUES = [
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
];

/** config.toml `personality` 支持的档位（codex-cli Personality）。
 *  friendly 亲和（默认，较啰嗦）、pragmatic 务实简洁、none 无人格；省输出 token 选 pragmatic。 */
const PERSONALITY_OPTIONS: AppSelectOption[] = [
  { value: "", label: "默认（不写入）" },
  { value: "friendly", label: "friendly（亲和）" },
  { value: "pragmatic", label: "pragmatic（务实简洁）" },
  { value: "none", label: "none（无人格）" },
];

/** config.toml `model_verbosity` 支持的档位（GPT-5 系输出详细程度）。
 *  控制最终回答的长短，不影响推理；省输出 token 选 low。 */
const VERBOSITY_OPTIONS: AppSelectOption[] = [
  { value: "", label: "默认（不写入）" },
  { value: "low", label: "low（简洁）" },
  { value: "medium", label: "medium（适中）" },
  { value: "high", label: "high（详细）" },
];

/** 模型配置卡片校验状态：空串表示无错误；catalog 为模型目录结构错误（阻断保存） */
const modelConfigErrors = reactive({
  model: "",
  provider: "",
  catalog: "",
});

/** 正在生成模型目录的提供方 key；空串表示没有生成任务。 */
const catalogGenerating = ref("");

/** 获取模型后选择要写入目录的模型。 */
const catalogPicker = reactive({
  open: false,
  providerName: "",
  baseUrl: "",
  total: 0,
  matched: 0,
  skipped: 0,
  catalog: "",
  models: [] as ModelCatalogModelOption[],
  selectedIds: [] as string[],
  query: "",
});

/** 候选模型的资料来源标记（只读展示，不影响勾选与生成流程）。 */
function catalogSourceLabel(model: ModelCatalogModelOption): string {
  const labels: string[] = [];
  if (model.official) labels.push("官方条目");
  if (model.models_dev) labels.push("models.dev");
  if (model.openrouter) labels.push("OpenRouter");
  return labels.join(" + ");
}

/** 提供方新增/编辑表单状态（editingIndex < 0 表示新增） */
const providerForm = reactive({
  open: false,
  editingIndex: -1,
  key: "",
  name: "",
  base_url: "",
  env_key: "",
  experimental_bearer_token: "",
  wire_api: "responses",
  requires_openai_auth: false,
});

/** 提供方表单逐字段校验状态 */
const providerFormErrors = reactive({
  key: "",
  name: "",
  base_url: "",
  auth: "",
});

/** 顶层 preferred_auth_method："" 默认（不写入）/ "apikey" / "chatgpt" */
const preferredAuthOptions: AppSelectOption[] = [
  { value: "", label: "默认（不写入）" },
  { value: "apikey", label: "apikey" },
  { value: "chatgpt", label: "ChatGPT/OpenAI 登录" },
];

/** 顶层 forced_login_method："" 默认（不写入）/ "api" / "chatgpt" */
const forcedAuthOptions: AppSelectOption[] = [
  { value: "", label: "默认（不写入）" },
  { value: "api", label: "api" },
  { value: "chatgpt", label: "ChatGPT/OpenAI 登录" },
];

/** AppSelect 下拉选项集（替换原生 select：弹出层可主题化/毛玻璃） */
const reasoningEffortOptions: AppSelectOption[] = [
  { value: "", label: "默认（不写入）" },
  ...REASONING_EFFORT_VALUES.map((v) => ({ value: v, label: v })),
];
// codex 0.149.x 起仅支持 responses（chat 会让整份配置失效）
const wireApiOptions: AppSelectOption[] = [
  { value: "responses", label: "responses" },
];

function clearProviderFormErrors() {
  providerFormErrors.key = "";
  providerFormErrors.name = "";
  providerFormErrors.base_url = "";
  providerFormErrors.auth = "";
}

/** 应用 config 卡片字段（刷新该卡片时不影响 model_catalog_json 文本框） */
function applyConfigCard(res: ModelConfigState) {
  modelConfig.config_path = res.config_path;
  modelConfig.config_exists = res.config_exists;
  modelConfig.config_content = res.config_content;
}

/** 应用 model_catalog_json 卡片字段 */
function applyCatalogCard(res: ModelConfigState) {
  modelConfig.model_catalog_path = res.model_catalog_path;
  modelConfig.model_catalog_exists = res.model_catalog_exists;
  modelConfig.model_catalog = res.model_catalog;
}

/** 应用「模型提供方」卡片字段（刷新该卡片时不影响其它卡片文本框） */
function applyProvidersCard(pc: ModelProviderConfigState) {
  modelConfig.model = pc.model;
  modelConfig.model_reasoning_effort = pc.model_reasoning_effort;
  modelConfig.personality = pc.personality;
  modelConfig.model_verbosity = pc.model_verbosity;
  modelConfig.model_provider = pc.model_provider;
  modelConfig.preferred_auth_method = pc.preferred_auth_method;
  modelConfig.forced_login_method = pc.forced_login_method;
  // 浅拷贝：组件内增删改不污染调用方数组引用（测试/热更新下尤其重要）
  modelConfig.providers = (pc.providers ?? []).map((p) => ({ ...p }));
}

onMounted(() => {
  void loadModelConfig();
});

async function loadModelConfig() {
  modelConfig.loading = true;
  try {
    const res = await invoke<ModelConfigState>("model_config_read");
    applyConfigCard(res);
    applyCatalogCard(res);
    // openai_api_key_present 是进程环境检查，config/read 不提供，由 model_config_read 补充
    modelConfig.openai_api_key_present = res.openai_api_key_present;
    const pc = await loadModelProviderConfig();
    applyProvidersCard(pc);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    modelConfig.loading = false;
  }
}

/** 打开「添加」提供方表单 */
function openAddProvider() {
  providerForm.open = true;
  providerForm.editingIndex = -1;
  providerForm.key = "";
  providerForm.name = "";
  providerForm.base_url = "";
  providerForm.env_key = "";
  providerForm.experimental_bearer_token = "";
  providerForm.wire_api = "responses";
  providerForm.requires_openai_auth = false;
  clearProviderFormErrors();
}

/** 打开「编辑提供方」表单（标识 key 只读） */
function openEditProvider(index: number) {
  const p = modelConfig.providers[index];
  if (!p) return;
  providerForm.open = true;
  providerForm.editingIndex = index;
  providerForm.key = p.key;
  providerForm.name = p.name;
  providerForm.base_url = p.base_url;
  providerForm.env_key = p.env_key;
  providerForm.experimental_bearer_token = p.experimental_bearer_token;
  // 仅 responses 合法：历史 chat 值在编辑时直接回填 responses，保存即修正
  providerForm.wire_api = "responses";
  providerForm.requires_openai_auth = !!p.requires_openai_auth;
  clearProviderFormErrors();
}

function closeProviderForm() {
  providerForm.open = false;
}

/** 提交提供方表单：校验后写入本地列表（保存按钮统一落盘） */
function confirmProviderForm() {
  clearProviderFormErrors();
  const key = providerForm.key.trim();
  if (!key) {
    providerFormErrors.key = "请填写提供方标识";
    return;
  }
  if (!/^[A-Za-z0-9_-]+$/.test(key)) {
    providerFormErrors.key = "只能包含字母、数字、下划线与连字符";
    return;
  }
  if (providerForm.editingIndex < 0) {
    if (modelConfig.providers.some((p) => p.key === key)) {
      providerFormErrors.key = "提供方标识已存在";
      return;
    }
  }
  if (!providerForm.name.trim()) {
    providerFormErrors.name = "请填写提供方名称";
    return;
  }
  if (!providerForm.base_url.trim()) {
    providerFormErrors.base_url = "请填写 base_url";
    return;
  }
  const hasAuth =
    !!providerForm.env_key.trim() ||
    !!providerForm.experimental_bearer_token.trim() ||
    providerForm.requires_openai_auth ||
    modelConfig.openai_api_key_present;
  if (!hasAuth) {
    providerFormErrors.auth =
      "请填写 env_key、API Key 或选择 ChatGPT/OpenAI 登录认证（也可仅设置全局 OPENAI_API_KEY）";
    return;
  }
  // 选择 ChatGPT/OpenAI 登录认证时不依赖 API Key，清空避免与登录路径产生矛盾配置
  if (providerForm.requires_openai_auth) {
    providerForm.env_key = "";
    providerForm.experimental_bearer_token = "";
  }
  if (providerForm.editingIndex < 0) {
    modelConfig.providers.push({
      key,
      name: providerForm.name.trim(),
      base_url: providerForm.base_url.trim(),
      env_key: providerForm.env_key.trim(),
      experimental_bearer_token: providerForm.experimental_bearer_token.trim(),
      wire_api: providerForm.wire_api,
      requires_openai_auth: providerForm.requires_openai_auth,
    });
    // 第一个提供方自动激活，减少新手配置步骤
    if (modelConfig.providers.length === 1) {
      modelConfig.model_provider = key;
    }
  } else {
    const p = modelConfig.providers[providerForm.editingIndex];
    if (!p) return;
    p.name = providerForm.name.trim();
    p.base_url = providerForm.base_url.trim();
    p.env_key = providerForm.env_key.trim();
    p.experimental_bearer_token = providerForm.experimental_bearer_token.trim();
    p.wire_api = providerForm.wire_api;
    p.requires_openai_auth = providerForm.requires_openai_auth;
  }
  providerForm.open = false;
}

/** 删除提供方：删除当前激活项时清空激活选择 */
function removeProvider(index: number) {
  const p = modelConfig.providers[index];
  if (!p) return;
  if (p.key === modelConfig.model_provider) {
    modelConfig.model_provider = "";
  }
  modelConfig.providers.splice(index, 1);
}

/** 仅当提供方同时有 base_url 和 API Key 时允许生成模型目录。 */
function canGenerateModelCatalog(p: ModelProviderInfo): boolean {
  return !!(p.base_url ?? "").trim() && !!(p.experimental_bearer_token ?? "").trim();
}

/** 按搜索条件过滤候选模型；未匹配项保留用于禁用展示。 */
const filteredCatalogModels = computed(() => {
  const query = catalogPicker.query.trim().toLowerCase();
  if (!query) return catalogPicker.models;
  return catalogPicker.models.filter(
    (model) =>
      model.id.toLowerCase().includes(query) ||
      model.display_name.toLowerCase().includes(query),
  );
});

const visibleMatchedCatalogModels = computed(() =>
  filteredCatalogModels.value.filter((model) => model.matched),
);

const allVisibleCatalogModelsSelected = computed(
  () =>
    visibleMatchedCatalogModels.value.length > 0 &&
    visibleMatchedCatalogModels.value.every((model) =>
      catalogPicker.selectedIds.includes(model.id),
    ),
);

const someVisibleCatalogModelsSelected = computed(() =>
  visibleMatchedCatalogModels.value.some((model) =>
    catalogPicker.selectedIds.includes(model.id),
  ),
);

/** 打开模型选择弹窗，默认不选择模型。 */
function openCatalogPicker(
  p: ModelProviderInfo,
  res: ModelCatalogGenerateResult,
) {
  catalogPicker.open = true;
  catalogPicker.providerName = p.name || p.key;
  catalogPicker.baseUrl = p.base_url.trim();
  catalogPicker.total = res.total;
  catalogPicker.matched = res.matched;
  catalogPicker.skipped = res.skipped;
  catalogPicker.catalog = res.catalog;
  catalogPicker.models = res.models.map((model) => ({ ...model }));
  catalogPicker.selectedIds = [];
  catalogPicker.query = "";
}

function closeCatalogPicker() {
  catalogPicker.open = false;
  catalogPicker.providerName = "";
  catalogPicker.baseUrl = "";
  catalogPicker.total = 0;
  catalogPicker.matched = 0;
  catalogPicker.skipped = 0;
  catalogPicker.catalog = "";
  catalogPicker.models = [];
  catalogPicker.selectedIds = [];
  catalogPicker.query = "";
}

/** 三态全选：全部选中时取消当前结果，否则选中当前结果。 */
function toggleVisibleCatalogModels() {
  const visibleIds = new Set(
    visibleMatchedCatalogModels.value.map((model) => model.id),
  );
  if (allVisibleCatalogModelsSelected.value) {
    catalogPicker.selectedIds = catalogPicker.selectedIds.filter(
      (id) => !visibleIds.has(id),
    );
    return;
  }
  for (const model of visibleMatchedCatalogModels.value) {
    if (!catalogPicker.selectedIds.includes(model.id)) {
      catalogPicker.selectedIds.push(model.id);
    }
  }
}

/** 用选中的模型覆盖编辑框；不自动保存。 */
function confirmCatalogPicker() {
  if (!catalogPicker.selectedIds.length) return;
  try {
    modelConfig.model_catalog = filterCatalogByModelIds(
      catalogPicker.catalog,
      catalogPicker.selectedIds,
    );
    modelConfigErrors.catalog = "";
    const skipped =
      catalogPicker.skipped > 0
        ? `，跳过 ${catalogPicker.skipped} 个未匹配模型`
        : "";
    setToast(
      `已生成 ${catalogPicker.selectedIds.length} 个模型条目${skipped}，保存并重启 codex-ui 后生效`,
    );
    closeCatalogPicker();
  } catch (e) {
    setToast(toastError(e));
  }
}

/** 从当前提供方的 `/models` 获取候选模型，打开选择弹窗；不自动保存。 */
async function generateModelCatalog(p: ModelProviderInfo) {
  if (
    modelConfig.loading ||
    modelConfig.saving ||
    catalogGenerating.value ||
    !canGenerateModelCatalog(p)
  ) {
    return;
  }
  catalogGenerating.value = p.key;
  try {
    const res = await invoke<ModelCatalogGenerateResult>(
      "model_catalog_generate_from_provider",
      {
        baseUrl: p.base_url.trim(),
        apiKey: p.experimental_bearer_token.trim(),
      },
    );
    openCatalogPicker(p, res);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    catalogGenerating.value = "";
  }
}

/** 校验单个提供方必填字段，返回错误文案（空串表示通过） */
function providerRowError(p: ModelProviderInfo): string {
  if (!(p.name ?? "").trim()) return "缺少名称（name）";
  if (!(p.base_url ?? "").trim()) return "缺少 base_url";
  if (!(p.wire_api ?? "").trim()) return "缺少 wire_api";
  // codex 0.149.x 仅支持 responses：历史 chat 值会让配置失效（用户层读不到、提供方消失）
  if ((p.wire_api ?? "").trim() !== "responses") {
    return `wire_api 仅支持 responses（当前 ${p.wire_api}）`;
  }
  if (
    !(p.env_key ?? "").trim() &&
    !(p.experimental_bearer_token ?? "").trim() &&
    !p.requires_openai_auth &&
    !modelConfig.openai_api_key_present
  ) {
    return "缺少认证方式（env_key / API Key / 全局 OPENAI_API_KEY）";
  }
  return "";
}

/** 由 model_catalog_json 编辑框内容提取模型 slug（缺失回退 id），供模型名输入建议 */
const catalogModelIds = computed<string[]>(() => {
  const content = (modelConfig.model_catalog ?? "").trim();
  if (!content) return [];
  try {
    const parsed: unknown = JSON.parse(content);
    const arr =
      parsed && typeof parsed === "object" && !Array.isArray(parsed)
        ? (parsed as { models?: unknown }).models
        : undefined;
    if (!Array.isArray(arr)) return [];
    const ids: string[] = [];
    for (const m of arr) {
      if (!m || typeof m !== "object") continue;
      const candid = m as Record<string, unknown>;
      const id = typeof candid.slug === "string" ? candid.slug : candid.id;
      if (typeof id === "string" && id.trim() && !ids.includes(id)) {
        ids.push(id.trim());
      }
    }
    return ids;
  } catch {
    return [];
  }
});

/** 保存模型配置：提供方 + 模型标量 + 模型目录合并为一个保存；目录内容非法时提示并阻断 */
async function saveModelConfig() {
  if (modelConfig.saving || modelConfig.loading) return;
  modelConfigErrors.model = "";
  modelConfigErrors.provider = "";
  modelConfigErrors.catalog = "";
  let blocked = false;
  for (const p of modelConfig.providers) {
    const err = providerRowError(p);
    if (err) {
      setToast(`提供方「${p.key}」${err}`);
      blocked = true;
    }
  }
  if (blocked) return;

  // 模型目录内容：非空时先做 JSON 合法性提示，再去 Rust 做结构校验并落盘
  const catalogContent = modelConfig.model_catalog.trim();
  if (catalogContent) {
    try {
      JSON.parse(catalogContent);
    } catch {
      modelConfigErrors.catalog = "模型目录不是合法 JSON";
      blocked = true;
    }
    if (!blocked) {
      try {
        await invoke("model_catalog_save", { content: catalogContent });
      } catch (e) {
        const msg = toastError(e);
        modelConfigErrors.catalog = msg;
        setToast(msg);
        blocked = true;
      }
    }
  }
  if (blocked) return;

  modelConfig.saving = true;
  try {
    const input: ModelConfigUiEdit = {
      model: modelConfig.model.trim(),
      model_reasoning_effort: modelConfig.model_reasoning_effort.trim(),
      personality: modelConfig.personality,
      model_verbosity: modelConfig.model_verbosity,
      model_provider: modelConfig.model_provider,
      preferred_auth_method: modelConfig.preferred_auth_method.trim(),
      forced_login_method: modelConfig.forced_login_method.trim(),
      model_catalog_json: catalogContent ? modelConfig.model_catalog_path : null,
      providers: modelConfig.providers,
    };
    await saveModelProviderConfig(input);
    setToast("模型配置已保存，重启 codex-ui 后生效");
    await loadModelConfig();
  } catch (e) {
    setToast(toastError(e));
  } finally {
    modelConfig.saving = false;
  }
}

/** 在编辑器中打开模型目录文件（model_catalog_json 目标路径）；失败回退资源管理器定位 */
function openModelConfigFile() {
  void openPathInAppOrReveal(modelConfig.model_catalog_path);
}
</script>

<template>
  <section v-show="active" class="settings-section settings-section-model-config">
    <h2 class="settings-section-title">模型配置</h2>
    <p class="settings-section-desc">
      模型、模型提供方、模型目录与模型快照
    </p>

    <ModelSnapshotCard :active="active" @applied="() => void loadModelConfig()" />

    <div class="model-config-card">
      <div class="model-config-card-head">
        <h3>模型配置</h3>
        <div class="model-config-head-actions">
          <button
            type="button"
            class="model-config-docs-link"
            v-tooltip="'DeepSeek 接入文档（浏览器打开）'"
            @click="openDocsUrl(DEEPSEEK_CODEX_DOCS_URL)"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_LINK" />
            </svg>
            DeepSeek
          </button>
          <button
            type="button"
            class="model-config-docs-link"
            v-tooltip="'GLM 接入文档（浏览器打开）'"
            @click="openDocsUrl(GLM_CODEX_DOCS_URL)"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_LINK" />
            </svg>
            GLM
          </button>
          <button
            class="btn btn-icon primary model-config-add-btn"
            v-tooltip="'添加模型提供方'"
            aria-label="添加模型提供方"
            :disabled="modelConfig.loading"
            @click="openAddProvider"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_PLUS" />
            </svg>
          </button>
          <button
            class="btn btn-icon model-config-reload-btn"
            aria-label="刷新"
            v-tooltip="'刷新'"
            :disabled="modelConfig.loading"
            @click="loadModelConfig"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_REFRESH" />
            </svg>
          </button>
        </div>
      </div>

      <div class="model-providers-list">
        <div class="model-provider-row model-provider-none">
          <label class="model-provider-radio">
            <input
              type="radio"
              name="model-provider-active"
              :value="''"
              v-model="modelConfig.model_provider"
              :disabled="modelConfig.loading"
            />
            <span class="model-provider-name">不使用提供者</span>
            <span class="model-provider-key">(none)</span>
          </label>
        </div>
        <div
          v-if="modelConfig.providers.length === 0"
          class="model-providers-empty"
        >
          还没有提供方，点击右上角「+」创建。
        </div>
        <div
          v-for="(p, i) in modelConfig.providers"
          :key="p.key"
          class="model-provider-row"
          :class="{ 'model-provider-row-error': providerRowError(p) }"
        >
          <label class="model-provider-radio">
            <input
              type="radio"
              name="model-provider-active"
              :value="p.key"
              v-model="modelConfig.model_provider"
              :disabled="modelConfig.loading"
            />
            <span class="model-provider-name">
              {{ p.name || p.key }}
              <span class="model-config-required" aria-label="必填" v-tooltip="'必填'">*</span>
            </span>
            <span class="model-provider-key">{{ p.key }}</span>
            <span v-if="p.wire_api" class="model-provider-wire">{{ p.wire_api }}</span>
          </label>
          <p
            v-if="providerRowError(p)"
            class="model-config-field-error model-provider-row-msg"
          >
            {{ providerRowError(p) }}
          </p>
          <div class="model-provider-actions">
            <button
              v-if="canGenerateModelCatalog(p)"
              class="btn btn-icon provider-row-generate"
              :disabled="modelConfig.loading || !!catalogGenerating"
              aria-label="生成模型目录"
              v-tooltip="'生成模型目录'"
              @click="generateModelCatalog(p)"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_ARROW_CIRCLE_RIGHT" />
              </svg>
            </button>
            <button
              class="btn btn-icon provider-row-edit"
              aria-label="编辑"
              v-tooltip="'编辑'"
              :disabled="modelConfig.loading"
              @click="openEditProvider(i)"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_EDIT" />
              </svg>
            </button>
            <button
              class="btn btn-icon danger provider-row-delete"
              :disabled="modelConfig.loading"
              aria-label="删除"
              v-tooltip="'删除'"
              @click="removeProvider(i)"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_DELETE" />
              </svg>
            </button>
          </div>
        </div>
        <p
          v-if="modelConfigErrors.provider"
          class="model-config-field-error"
        >
          {{ modelConfigErrors.provider }}
        </p>
      </div>

      <div class="settings">
        <div
          class="setting-row"
          :class="{ 'model-config-row-error': modelConfigErrors.model }"
        >
          <label for="model-config-ui-model">
            model（模型标识）
          </label>
          <ModelConfigModelPicker
            id="model-config-ui-model"
            v-model="modelConfig.model"
            :options="catalogModelIds"
            :disabled="modelConfig.loading"
            :error="!!modelConfigErrors.model"
          />
          <p
            v-if="modelConfigErrors.model"
            class="model-config-field-error"
          >
            {{ modelConfigErrors.model }}
          </p>
        </div>
        <div class="setting-row">
          <label for="model-config-ui-effort">model_reasoning_effort（推理强度）</label>
          <AppSelect
            id="model-config-ui-effort"
            v-model="modelConfig.model_reasoning_effort"
            :disabled="modelConfig.loading"
            :options="reasoningEffortOptions"
          />
        </div>
        <div class="setting-row">
          <label for="model-config-ui-auth">preferred_auth_method（优先认证方式）</label>
          <AppSelect
            id="model-config-ui-auth"
            v-model="modelConfig.preferred_auth_method"
            :disabled="modelConfig.loading"
            :options="preferredAuthOptions"
          />
        </div>
        <div class="setting-row">
          <label for="model-config-ui-forced">forced_login_method（强制登录方式）</label>
          <AppSelect
            id="model-config-ui-forced"
            v-model="modelConfig.forced_login_method"
            :disabled="modelConfig.loading"
            :options="forcedAuthOptions"
          />
        </div>
        <div class="setting-row">
          <label for="model-config-ui-personality">personality（回复风格）</label>
          <AppSelect
            id="model-config-ui-personality"
            v-model="modelConfig.personality"
            :disabled="modelConfig.loading"
            :options="PERSONALITY_OPTIONS"
          />
        </div>
        <div class="setting-row">
          <label for="model-config-ui-verbosity">model_verbosity（输出详细程度）</label>
          <AppSelect
            id="model-config-ui-verbosity"
            v-model="modelConfig.model_verbosity"
            :disabled="modelConfig.loading"
            :options="VERBOSITY_OPTIONS"
          />
        </div>
      </div>

      <div class="model-catalog-block">
        <div class="model-catalog-head">
          <span>model_catalog_json（模型目录）</span>
        </div>
        <button
          v-if="modelConfig.model_catalog_exists"
          type="button"
          class="model-config-path-link"
          v-tooltip="'在编辑器中打开文件'"
          :aria-label="`在编辑器中打开 ${modelConfig.model_catalog_path}`"
          @click="openModelConfigFile"
        >
          <span>{{ modelConfig.model_catalog_path || "正在读取目录路径…" }}</span>
        </button>
        <p v-else class="model-config-path">
          {{ modelConfig.model_catalog_path || "正在读取目录路径…" }}
        </p>
        <textarea
          v-model="modelConfig.model_catalog"
          class="model-config-textarea"
          :disabled="modelConfig.loading"
          placeholder='在此编辑模型目录内容（必须为合法 JSON，如 {"models":[]}）'
          spellcheck="false"
        ></textarea>
        <p v-if="modelConfigErrors.catalog" class="model-config-field-error">
          {{ modelConfigErrors.catalog }}
        </p>
      </div>

      <ModalDialog
        v-if="providerForm.open"
        :title="providerForm.editingIndex >= 0 ? '编辑模型提供方' : '添加模型提供方'"
        closable
        @close="closeProviderForm"
      >
        <div class="model-provider-form">
          <div
            class="setting-row"
            :class="{ 'model-config-row-error': providerFormErrors.key }"
          >
            <label>
              key（标识）
              <span class="model-config-required" aria-label="必填" v-tooltip="'必填'">*</span>
            </label>
            <input
              v-model="providerForm.key"
              type="text"
              :disabled="providerForm.editingIndex >= 0"
              placeholder="如 my-provider"
              :class="{ 'model-config-input-error': providerFormErrors.key }"
            />
            <p
              v-if="providerFormErrors.key"
              class="model-config-field-error"
            >
              {{ providerFormErrors.key }}
            </p>
          </div>
          <div
            class="setting-row"
            :class="{ 'model-config-row-error': providerFormErrors.name }"
          >
            <label>
              name（名称）
              <span class="model-config-required" aria-label="必填" v-tooltip="'必填'">*</span>
            </label>
            <input
              v-model="providerForm.name"
              type="text"
              placeholder="如 DeepSeek"
              :class="{ 'model-config-input-error': providerFormErrors.name }"
            />
            <p
              v-if="providerFormErrors.name"
              class="model-config-field-error"
            >
              {{ providerFormErrors.name }}
            </p>
          </div>
          <div
            class="setting-row"
            :class="{ 'model-config-row-error': providerFormErrors.base_url }"
          >
            <label>
              base_url（接口地址）
              <span class="model-config-required" aria-label="必填" v-tooltip="'必填'">*</span>
            </label>
            <input
              v-model="providerForm.base_url"
              type="text"
              placeholder="https://api.example.com/v1"
              :class="{
                'model-config-input-error': providerFormErrors.base_url,
              }"
            />
            <p
              v-if="providerFormErrors.base_url"
              class="model-config-field-error"
            >
              {{ providerFormErrors.base_url }}
            </p>
          </div>
          <div class="setting-row">
            <div class="checkbox-row">
              <input
                id="provider-openai-auth"
                v-model="providerForm.requires_openai_auth"
                type="checkbox"
              />
              <label for="provider-openai-auth">
                requires_openai_auth（ChatGPT/OpenAI 登录认证）
              </label>
            </div>
          </div>
          <div v-if="!providerForm.requires_openai_auth" class="setting-row">
            <label>env_key（环境变量名）</label>
            <input
              v-model="providerForm.env_key"
              type="text"
              placeholder="如 OPENAI_API_KEY"
            />
          </div>
          <div
            v-if="!providerForm.requires_openai_auth"
            class="setting-row"
            :class="{ 'model-config-row-error': providerFormErrors.auth }"
          >
            <label>experimental_bearer_token（API Key，一般是 sk-* 前缀）</label>
            <input
              v-model="providerForm.experimental_bearer_token"
              type="password"
              placeholder="API Key"
            />
            <p
              v-if="providerFormErrors.auth"
              class="model-config-field-error"
            >
              {{ providerFormErrors.auth }}
            </p>
          </div>
          <div class="setting-row">
            <label>wire_api（接口协议）</label>
            <AppSelect v-model="providerForm.wire_api" :options="wireApiOptions" />
          </div>
        </div>
        <template #foot>
          <button class="btn" @click="closeProviderForm">取消</button>
          <button
            class="btn primary provider-form-submit"
            :disabled="modelConfig.saving || modelConfig.loading"
            @click="confirmProviderForm"
          >
            确认
          </button>
        </template>
      </ModalDialog>

      <ModalDialog
        v-if="catalogPicker.open"
        title="选择要生成的模型"
        body-class="model-catalog-picker-body"
        closable
        @close="closeCatalogPicker"
      >
        <div class="model-catalog-picker">
          <div class="model-catalog-picker-meta">
            <div class="model-catalog-picker-provider">
              <span>{{ catalogPicker.providerName }}</span>
              <span class="model-catalog-picker-url">{{ catalogPicker.baseUrl }}</span>
            </div>
            <p>
              获取 {{ catalogPicker.total }} 个模型，可生成
              {{ catalogPicker.matched }} 个；{{ catalogPicker.skipped }}
              个未匹配资料，将跳过。
            </p>
          </div>
          <div class="model-catalog-picker-toolbar">
            <input
              v-model="catalogPicker.query"
              type="search"
              class="model-catalog-picker-search"
              placeholder="搜索模型 ID 或显示名..."
            />
            <label
              class="model-catalog-picker-select-all"
              :class="{
                'is-disabled': visibleMatchedCatalogModels.length === 0,
              }"
            >
              <input
                type="checkbox"
                :checked="allVisibleCatalogModelsSelected"
                :indeterminate="
                  someVisibleCatalogModelsSelected &&
                  !allVisibleCatalogModelsSelected
                "
                :disabled="visibleMatchedCatalogModels.length === 0"
                aria-label="全选当前搜索结果"
                @change="toggleVisibleCatalogModels"
              />
              <span>全选</span>
            </label>
          </div>
          <div class="model-catalog-picker-count">
            已选 {{ catalogPicker.selectedIds.length }} / 可生成
            {{ catalogPicker.matched }}
          </div>
          <div class="model-catalog-picker-list">
            <label
              v-for="model in filteredCatalogModels"
              :key="model.id"
              class="model-catalog-picker-row"
              :class="{ 'is-unmatched': !model.matched }"
            >
              <input
                type="checkbox"
                :value="model.id"
                v-model="catalogPicker.selectedIds"
                :disabled="!model.matched"
              />
              <span
                class="model-catalog-picker-text"
                @click.stop.prevent
              >
                <span class="model-catalog-picker-id">{{ model.id }}</span>
                <span class="model-catalog-picker-name">{{ model.display_name }}</span>
              </span>
              <span
                v-if="model.matched"
                class="model-catalog-picker-source"
                @click.stop.prevent
              >
                {{ catalogSourceLabel(model) }}
              </span>
              <span
                v-if="!model.matched"
                class="model-catalog-picker-status"
                @click.stop.prevent
              >
                无匹配资料，将跳过
              </span>
            </label>
            <p
              v-if="filteredCatalogModels.length === 0"
              class="model-catalog-picker-empty"
            >
              没有匹配的模型
            </p>
          </div>
        </div>
        <template #foot>
          <button class="btn" type="button" @click="closeCatalogPicker">
            取消
          </button>
          <button
            class="btn model-catalog-picker-confirm"
            type="button"
            :disabled="catalogPicker.selectedIds.length === 0"
            @click="confirmCatalogPicker"
          >
            生成 {{ catalogPicker.selectedIds.length }} 个条目
          </button>
        </template>
      </ModalDialog>

      <div class="model-config-actions">
        <button
          class="btn model-config-save-btn"
          :class="{ loading: modelConfig.saving }"
          aria-label="保存"
          :disabled="modelConfig.saving || modelConfig.loading"
          @click="saveModelConfig"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_SAVE" />
          </svg>
          <span>保存</span>
        </button>
      </div>
    </div>
  </section>
</template>

<style scoped>
.model-config-path {
  font-family: var(--mono);
  font-size: var(--font-sm);
  line-height: 1.4;
  color: var(--text-dim);
  word-break: break-all;
  user-select: text;
  text-align: right;
  min-width: 0;
}

.model-config-path-link:hover {
  color: var(--accent-dim);
}

/* 模型目录路径链接：作为 column 容器 item 默认会被拉伸占满整行，
   导致 tooltip 相对整行居中而偏到路径名右侧；限制为内容宽度使其只围绕路径名显示 */
.model-config-path-link {
  align-self: flex-start;
}

/* 模型提供方列表与表单 */
.model-providers-list {
  display: flex;
  flex-direction: column;
}

.model-provider-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  padding: var(--space-4) 0;
  border-bottom: 1px solid var(--border);
  transition: background var(--ease);
}

.model-provider-row:hover {
  background: rgba(var(--overlay-rgb), 0.025);
}

.model-provider-row:last-child {
  border-bottom: none;
}

.model-provider-none .model-provider-name {
  color: var(--text-dim);
}

.model-provider-radio {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex: 1;
  min-width: 0;
  cursor: pointer;
}

.model-provider-radio input[type="radio"] {
  accent-color: var(--accent);
  cursor: pointer;
}

.model-provider-name {
  font-size: var(--font-base);
  color: var(--text-bright);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.model-provider-key {
  font-size: var(--font-sm);
  color: var(--text-dim);
  font-family: var(--mono);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.model-provider-wire {
  padding: 1px var(--space-2);
}

.model-providers-empty {
  padding: var(--space-8);
  text-align: center;
  font-size: var(--font-md);
  color: var(--text-dim);
  background: rgba(var(--overlay-rgb), 0.02);
  border: 1px dashed var(--border);
  border-radius: var(--radius-lg);
}

.model-provider-form {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: var(--space-6) var(--space-8);
  background: rgba(var(--overlay-rgb), 0.02);
}

.model-provider-row-error {
  border: 1px solid rgba(var(--red-rgb), 0.35);
  border-radius: var(--radius);
  background: rgba(var(--red-rgb), 0.06);
  padding: var(--space-2) var(--space-3);
}

.model-provider-row-msg {
  flex: 0 0 100%;
}

.checkbox-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.checkbox-row label {
  margin: 0;
  font-size: var(--font-md);
  font-weight: 400;
  line-height: 1.5;
  color: var(--text);
}

.checkbox-row input[type="checkbox"] {
  accent-color: var(--accent);
  width: 16px;
  height: 16px;
  cursor: pointer;
}

.model-config-textarea {
  width: 100%;
  min-height: 220px;
  resize: none;
  box-sizing: border-box;
  font-family: var(--mono);
  font-size: var(--font-md);
  line-height: 1.5;
  color: var(--text);
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: var(--space-3) var(--space-4);
}

.model-config-textarea:focus {
  outline: none;
  border-color: var(--accent);
}

/* 模型目录区块：只读路径 + JSON 编辑器（与提供方输入区之间加分隔） */
.model-catalog-block {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding-top: var(--space-5);
  border-top: 1px solid var(--border);
}

.model-catalog-head span {
  font-size: var(--font-sm);
  font-weight: 700;
  color: var(--text-bright);
}

.model-catalog-block .model-config-path {
  text-align: left;
}
</style>
