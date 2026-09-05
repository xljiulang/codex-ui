<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  askConfirm,
  loadMemoryConfig,
  loadModelProviderConfig,
  saveMemoryConfig,
  saveModelProviderConfig,
  saveSettings,
  setToast,
  store,
  toastError,
} from "../composables/useCodex";
import type {
  ModelProviderConfigState,
} from "../composables/useCodex";
import {
  THEMES,
  previewTheme,
  type ThemeId,
} from "../composables/useTheme";
import { openPathInApp } from "../composables/useSessionFs";
import {
  ICON_CHECK,
  ICON_DELETE,
  ICON_EDIT,
  ICON_EXTENSION,
  ICON_FILE,
  ICON_FOLDER_OPEN,
  ICON_LINK,
  ICON_MCP,
  ICON_MODEL_CUBE,
  ICON_PALETTE,
  ICON_PLUS,
  ICON_REFRESH,
  ICON_SAVE,
  ICON_SKILL,
  ICON_TOOL,
  ICON_TUNE,
} from "../lib/icons";
import { PERMISSION_MODES } from "../lib/permissions";
import type {
  AppSettings,
  ModelConfigUiEdit,
  ModelConfigState,
  ModelProviderInfo,
  TerminalShell,
  FollowupMode,
  PermissionId,
} from "../lib/types";
import AppSelect, { type AppSelectOption } from "./AppSelect.vue";
import ModelConfigModelPicker from "./ModelConfigModelPicker.vue";
import ModalDialog from "./ModalDialog.vue";
import DynamicToolsSection from "./settings/DynamicToolsSection.vue";
import GlobalInstructionsSection from "./settings/GlobalInstructionsSection.vue";
import McpSection from "./settings/McpSection.vue";
import PluginsSection from "./settings/PluginsSection.vue";
import SkillsSection from "./settings/SkillsSection.vue";

const codexPath = ref(store.settings.codex_path ?? "");
const sound = ref(store.settings.sound_enabled);
const enterToSend = ref(store.settings.enter_to_send);
const followupMode = ref(store.settings.followup_mode);
const theme = ref<ThemeId>(store.settings.theme as ThemeId);
const defaultPermission = ref(store.settings.default_permission);
const terminalShell = ref<TerminalShell>(store.settings.terminal_shell);
const glass = ref(store.settings.glass_effect);
/** 设置分类（左侧纵向导航；后续新增大类只需在此追加并补充右侧内容区） */
const settingsSectionIds = [
  "personalization",
  "basic",
  "global-instructions",
  "model-config",
  "dynamic-tools",
  "skills",
  "mcp",
  "plugins",
] as const;
type SettingsSectionId = (typeof settingsSectionIds)[number];
interface SettingsSection {
  id: SettingsSectionId;
  label: string;
  icon: string;
  /** 是否以描边渲染（当前仅「模型配置」用立方体线框） */
  stroke?: boolean;
}
const settingsSections: SettingsSection[] = [
  { id: "personalization", label: "个性化", icon: ICON_PALETTE },
  { id: "basic", label: "基础设置", icon: ICON_TUNE },
  { id: "global-instructions", label: "全局指令", icon: ICON_FILE },
  { id: "model-config", label: "模型配置", icon: ICON_MODEL_CUBE, stroke: true },
  { id: "dynamic-tools", label: "动态工具", icon: ICON_TOOL },
  { id: "skills", label: "技能管理", icon: ICON_SKILL },
  { id: "mcp", label: "MCP管理", icon: ICON_MCP },
  { id: "plugins", label: "插件管理", icon: ICON_EXTENSION },
];
/** 当前选中分类：默认取第一个分类（不依赖具体标签）；设置标签存在期间保持状态，关闭后重开才重置 */
const activeSection = ref<SettingsSectionId>(settingsSections[0].id);

/** 导航键盘操作：上下方向键循环切换分类（与其它面板方向键习惯一致） */
function onNavKeydown(e: KeyboardEvent) {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
  e.preventDefault();
  const idx = settingsSections.findIndex((s) => s.id === activeSection.value);
  const delta = e.key === "ArrowDown" ? 1 : -1;
  activeSection.value =
    settingsSections[(idx + delta + settingsSections.length) % settingsSections.length].id;
}

onMounted(() => {
  void loadModelConfig();
});

/** 即时保存：任何设置项变更立即持久化（成功静默，失败 toast） */
async function persist(patch: Partial<AppSettings>) {
  try {
    await saveSettings(patch);
  } catch (e) {
    setToast(toastError(e));
  }
}

async function pickCodexFile() {
  try {
    const current = codexPath.value.trim();
    const initialDir = current
      ? current.replace(/[\\/][^\\/]*$/, "")
      : undefined;
    const dir = await invoke<string | null>("pick_codex_file", {
      initialDir,
    });
    if (dir) {
      codexPath.value = dir;
      await persist({ codex_path: dir });
    }
  } catch (e) {
    setToast(toastError(e));
  }
}

/** 清除 codex 路径并即时保存为 null（恢复自动查找） */
async function clearCodexPath() {
  codexPath.value = "";
  await persist({ codex_path: null });
}

const memEnable = ref(false);
const memAllowTool = ref(false);

/** 读取 codex 配置回填记忆开关（进入「基础设置」标签时） */
async function loadMemorySection() {
  try {
    const s = await loadMemoryConfig();
    memEnable.value = s.enable;
    memAllowTool.value = s.allowToolGenerate;
  } catch (e) {
    setToast(toastError(e));
  }
}

/** 任一记忆开关变更：写回 codex 配置 */
async function saveMemorySection() {
  try {
    await saveMemoryConfig({
      enable: memEnable.value,
      allowToolGenerate: memAllowTool.value,
    });
  } catch (e) {
    setToast(toastError(e));
  }
}

// 进入「基础设置」标签时从 codex 配置回填开关（插件分区自己的限高补算在 PluginsSection 内）
watch(activeSection, (id) => {
  if (id === "basic") void loadMemorySection();
});

async function resetMemory() {
  const ok = await askConfirm({
    title: "删除记忆",
    message: "将清空全部已保存的记忆，且无法撤销。是否继续？",
    confirmLabel: "删除记忆",
    cancelLabel: "取消",
  });
  if (!ok) return;
  try {
    await invoke("codex_rpc", { method: "memory/reset", params: null });
    setToast("记忆已删除");
  } catch (e) {
    setToast(toastError(e));
  }
}

function selectTheme(id: ThemeId) {
  // 即时预览 + 立即持久化（saveSettings 内部 applyTheme 兜底一致）
  theme.value = id;
  previewTheme(id);
  void persist({ theme: id });
}

// ---------- 模型配置 ----------

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

/** 模型配置卡片校验状态：空串表示无错误；catalog 为模型目录结构错误（阻断保存） */
const modelConfigErrors = reactive({
  model: "",
  provider: "",
  catalog: "",
});

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
});

/** 提供方表单逐字段校验状态 */
const providerFormErrors = reactive({
  key: "",
  name: "",
  base_url: "",
  auth: "",
});

/** 顶层认证方式："" 默认（不写入）/ "apikey" / "chatgpt" */
const authMethod = ref("");

/** AppSelect 下拉选项集（替换原生 select：弹出层可主题化/毛玻璃） */
const reasoningEffortOptions: AppSelectOption[] = [
  { value: "", label: "默认（不写入）" },
  ...REASONING_EFFORT_VALUES.map((v) => ({ value: v, label: v })),
];
const authMethodOptions: AppSelectOption[] = [
  { value: "", label: "默认（不写入）" },
  { value: "apikey", label: "API Key" },
  { value: "chatgpt", label: "ChatGPT 登录" },
];
const wireApiOptions: AppSelectOption[] = [
  { value: "responses", label: "responses" },
  { value: "chat", label: "chat" },
];
const terminalShellOptions: AppSelectOption[] = [
  { value: "cmd", label: "cmd（命令提示符）" },
  { value: "powershell", label: "PowerShell" },
];
const defaultPermissionOptions: AppSelectOption[] = PERMISSION_MODES.map(
  (m) => ({ value: m.id, label: m.label }),
);
const followupModeOptions: AppSelectOption[] = [
  { value: "adjust", label: "调整方向" },
  { value: "queue", label: "加入队列" },
];

/** 写回持久化：AppSelect 回传 string，此处收窄为协议联合类型 */
function onTerminalShellChange(v: string) {
  terminalShell.value = v as TerminalShell;
  persist({ terminal_shell: terminalShell.value });
}
function onDefaultPermissionChange(v: string) {
  defaultPermission.value = v as PermissionId;
  persist({ default_permission: defaultPermission.value });
}
function onFollowupModeChange(v: string) {
  followupMode.value = v as FollowupMode;
  persist({ followup_mode: followupMode.value });
}

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
  modelConfig.model_provider = pc.model_provider;
  modelConfig.preferred_auth_method = pc.preferred_auth_method;
  modelConfig.forced_login_method = pc.forced_login_method;
  // 浅拷贝：组件内增删改不污染调用方数组引用（测试/热更新下尤其重要）
  modelConfig.providers = (pc.providers ?? []).map((p) => ({ ...p }));
  // 由 preferred_auth_method/forced_login_method 推导「认证方式」下拉
  if (
    pc.preferred_auth_method === "apikey" &&
    pc.forced_login_method === "api"
  ) {
    authMethod.value = "apikey";
  } else if (
    pc.preferred_auth_method === "chatgpt" &&
    pc.forced_login_method === "chatgpt"
  ) {
    authMethod.value = "chatgpt";
  } else {
    authMethod.value = "";
  }
}

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
  providerForm.wire_api = p.wire_api || "responses";
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
    modelConfig.openai_api_key_present;
  if (!hasAuth) {
    providerFormErrors.auth =
      "请填写 env_key 或 API Key（也可仅设置全局 OPENAI_API_KEY 环境变量）";
    return;
  }
  if (providerForm.editingIndex < 0) {
    modelConfig.providers.push({
      key,
      name: providerForm.name.trim(),
      base_url: providerForm.base_url.trim(),
      env_key: providerForm.env_key.trim(),
      experimental_bearer_token: providerForm.experimental_bearer_token.trim(),
      wire_api: providerForm.wire_api,
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
  }
  providerForm.open = false;
}

/** 删除提供方：激活项禁止删除 */
function removeProvider(index: number) {
  const p = modelConfig.providers[index];
  if (!p) return;
  if (p.key === modelConfig.model_provider) {
    setToast("请先切换到其它提供方，再删除当前激活项");
    return;
  }
  modelConfig.providers.splice(index, 1);
}

/** 校验单个提供方必填字段，返回错误文案（空串表示通过） */
function providerRowError(p: ModelProviderInfo): string {
  if (!(p.name ?? "").trim()) return "缺少名称（name）";
  if (!(p.base_url ?? "").trim()) return "缺少 base_url";
  if (!(p.wire_api ?? "").trim()) return "缺少 wire_api";
  if (
    !(p.env_key ?? "").trim() &&
    !(p.experimental_bearer_token ?? "").trim() &&
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

/** 认证方式下拉 → 顶层两个键（默认不写入） */
function authMethodToEdit(): {
  preferred_auth_method: string;
  forced_login_method: string;
} {
  if (authMethod.value === "apikey") {
    return { preferred_auth_method: "apikey", forced_login_method: "api" };
  }
  if (authMethod.value === "chatgpt") {
    return {
      preferred_auth_method: "chatgpt",
      forced_login_method: "chatgpt",
    };
  }
  return { preferred_auth_method: "", forced_login_method: "" };
}

/** 保存模型配置：提供方 + 模型标量 + 模型目录合并为一个保存；目录内容非法时提示并阻断 */
async function saveModelConfig() {
  if (modelConfig.saving || modelConfig.loading) return;
  modelConfigErrors.model = "";
  modelConfigErrors.provider = "";
  modelConfigErrors.catalog = "";
  let blocked = false;
  if (!modelConfig.model.trim()) {
    modelConfigErrors.model = "请填写 model (slug)";
    blocked = true;
  }
  if (modelConfig.providers.length > 0 && !modelConfig.model_provider.trim()) {
    modelConfigErrors.provider = "请选择一个模型提供方";
    blocked = true;
  }
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
    const auth = authMethodToEdit();
    const input: ModelConfigUiEdit = {
      model: modelConfig.model.trim(),
      model_reasoning_effort: modelConfig.model_reasoning_effort.trim(),
      model_provider: modelConfig.model_provider,
      preferred_auth_method: auth.preferred_auth_method,
      forced_login_method: auth.forced_login_method,
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

/** 应用内打开文件（文本走编辑器标签），失败回退资源管理器定位 */
async function openPathInAppOrReveal(path: string) {
  if (!path) return;
  const opened = await openPathInApp(path);
  if (opened) return;
  try {
    await invoke("reveal_path", { path });
  } catch (e) {
    setToast(toastError(e));
  }
}

/** 打开 Codex 接入文档（默认浏览器） */
function openDocs(url: string) {
  void invoke("open_url", { url }).catch(() => undefined);
}

</script>

<template>
  <div class="settings-page">
    <div class="settings-page-body">
      <nav
        class="settings-nav"
        aria-label="设置分类"
        @keydown="onNavKeydown"
      >
        <button
          v-for="s in settingsSections"
          :key="s.id"
          class="settings-nav-item"
          :class="{ active: activeSection === s.id }"
          :aria-pressed="activeSection === s.id"
          @click="activeSection = s.id"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              :d="s.icon"
              :fill="s.stroke ? 'none' : 'currentColor'"
              :stroke="s.stroke ? 'currentColor' : 'none'"
              :stroke-width="s.stroke ? 1.5 : undefined"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
          {{ s.label }}
        </button>
      </nav>
      <div class="settings-panel">
        <section
          v-show="activeSection === 'personalization'"
          class="settings-section settings-section-personalization"
        >
          <h2 class="settings-section-title">个性化</h2>
          <p class="settings-section-desc">
            主题、音效与消息发送等个性化偏好
          </p>
          <div class="settings-card">
            <div class="settings">
              <div class="setting-row checkbox-row">
                <input
                  id="sound"
                  v-model="sound"
                  type="checkbox"
                  @change="persist({ sound_enabled: sound })"
                />
                <label for="sound">提权/交互时播放提示音</label>
              </div>

              <div class="setting-row checkbox-row">
                <input
                  id="enter"
                  v-model="enterToSend"
                  type="checkbox"
                  @change="persist({ enter_to_send: enterToSend })"
                />
                <label for="enter">
                  Enter 快捷发送（开启时 Ctrl+Enter 换行；关闭后 Enter 换行，Ctrl+Enter 发送）
                </label>
              </div>

              <div class="setting-row">
                <div class="theme-row-head">
                  <label>毛玻璃主题外观</label>
                  <label
                    class="switch"
                    aria-label="毛玻璃特效"
                  >
                    <input
                      id="glass"
                      v-model="glass"
                      type="checkbox"
                      @change="persist({ glass_effect: glass })"
                    />
                    <span class="switch-track"></span>
                  </label>
                </div>
                <div class="theme-picker">
                  <button
                    v-for="t in THEMES"
                    :key="t.id"
                    class="theme-card"
                    :class="{ selected: theme === t.id }"
                    :data-theme-id="t.id"
                    :aria-pressed="theme === t.id"
                    @click="selectTheme(t.id)"
                  >
                    <span class="theme-swatch"></span>
                    <span class="theme-name">{{ t.name }}</span>
                    <span class="theme-desc">{{ t.desc }}</span>
                    <span v-if="theme === t.id" class="theme-check" aria-hidden="true">
                      <svg viewBox="0 0 24 24">
                        <path :d="ICON_CHECK" />
                      </svg>
                    </span>
                  </button>
                </div>
              </div>
            </div>
          </div>
        </section>

        <section
          v-show="activeSection === 'model-config'"
          class="settings-section settings-section-model-config"
        >
          <h2 class="settings-section-title">模型配置</h2>
          <p class="settings-section-desc">
            模型、模型提供方与模型目录
          </p>

          <div class="model-config-card">
            <div class="model-config-card-head">
              <h3>模型配置</h3>
              <div class="model-config-head-actions">
                <button
                  type="button"
                  class="model-config-docs-link"
                  v-tooltip="'DeepSeek 接入文档（浏览器打开）'"
                  @click="openDocs(DEEPSEEK_CODEX_DOCS_URL)"
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
                  @click="openDocs(GLM_CODEX_DOCS_URL)"
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
              <div v-if="modelConfig.providers.length === 0" class="model-providers-empty">
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
                    :disabled="modelConfig.loading || p.key === modelConfig.model_provider"
                    :aria-label="
                      p.key === modelConfig.model_provider
                        ? '先切换到其它提供方再删除'
                        : '删除'
                    "
                    v-tooltip="
                      p.key === modelConfig.model_provider
                        ? '先切换到其它提供方再删除'
                        : '删除'
                    "
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
                  model (slug)
                  <span class="model-config-required" aria-label="必填" v-tooltip="'必填'">*</span>
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
                <label for="model-config-ui-effort">model_reasoning_effort</label>
                <AppSelect
                  id="model-config-ui-effort"
                  v-model="modelConfig.model_reasoning_effort"
                  :disabled="modelConfig.loading"
                  :options="reasoningEffortOptions"
                />
              </div>
              <div class="setting-row">
                <label for="model-config-ui-auth">认证方式</label>
                <AppSelect
                  id="model-config-ui-auth"
                  v-model="authMethod"
                  :disabled="modelConfig.loading"
                  :options="authMethodOptions"
                />
                <p class="model-config-advanced-note">
                  选「API Key」写入
                  preferred_auth_method="apikey" 与 forced_login_method="api"
                </p>
              </div>
            </div>

            <div class="model-catalog-block">
              <div class="model-catalog-head">
                <span>模型目录（model_catalog_json）</span>
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
                    标识（key）
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
                    名称（name）
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
                    base_url
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
                  <label>env_key（环境变量名）</label>
                  <input
                    v-model="providerForm.env_key"
                    type="text"
                    placeholder="如 OPENAI_API_KEY"
                  />
                </div>
                <div
                  class="setting-row"
                  :class="{ 'model-config-row-error': providerFormErrors.auth }"
                >
                  <label>experimental_bearer_token</label>
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
                  <p
                    v-else-if="modelConfig.openai_api_key_present"
                    class="model-config-auth-hint"
                  >
                    已检测到全局 OPENAI_API_KEY，env_key / API Key 可留空
                  </p>
                </div>
                <div class="setting-row">
                  <label>wire_api</label>
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

            <div class="model-config-actions">
              <button
                class="btn primary model-config-save-btn"
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

        <GlobalInstructionsSection :active="activeSection === 'global-instructions'" />

        <section
          v-show="activeSection === 'basic'"
          class="settings-section settings-section-basic"
        >
          <h2 class="settings-section-title">基础设置</h2>
          <p class="settings-section-desc">
            终端、权限、跟进处理与本地记忆等基础设置
          </p>
          <div class="settings-card">
            <div class="settings">
              <div class="setting-row">
                <label>codex 可执行文件（留空自动查找）</label>
                <div class="setting-path-row codex-path-row">
                  <div class="setting-value codex-path-value">
                    {{ codexPath || "未设置（自动查找）" }}
                  </div>
                  <button
                    class="btn btn-icon codex-pick-btn"
                    v-tooltip="'选择文件'"
                    aria-label="选择文件"
                    @click="pickCodexFile()"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_FOLDER_OPEN" />
                    </svg>
                  </button>
                  <button
                    v-if="codexPath"
                    class="btn btn-icon danger codex-clear-btn"
                    v-tooltip="'清除'"
                    aria-label="清除"
                    @click="clearCodexPath()"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_DELETE" />
                    </svg>
                  </button>
                </div>
                <p v-if="!codexPath && store.server.codexPath" class="setting-note">
                  当前使用（自动检测）：{{ store.server.codexPath }}
                </p>
              </div>

              <div class="setting-row">
                <label>终端 Shell</label>
                <AppSelect
                  :model-value="terminalShell"
                  class="terminal-shell-select"
                  :options="terminalShellOptions"
                  @update:model-value="onTerminalShellChange"
                />
              </div>

              <div class="setting-row">
                <label>默认权限</label>
                <AppSelect
                  :model-value="defaultPermission"
                  class="default-permission-select"
                  :options="defaultPermissionOptions"
                  @update:model-value="onDefaultPermissionChange"
                />
              </div>

              <div class="setting-row">
                <label>跟进处理方式</label>
                <AppSelect
                  :model-value="followupMode"
                  :options="followupModeOptions"
                  @update:model-value="onFollowupModeChange"
                />
              </div>

              <div class="setting-row memory-row">
                <div class="memory-row-main">
                  <div class="memory-row-title">启用本地记忆</div>
                  <div class="memory-row-desc">根据此电脑上的聊天创建记忆，并用于个性化此电脑上的未来聊天</div>
                </div>
                <label class="switch">
                  <input type="checkbox" v-model="memEnable" @change="saveMemorySection()" />
                  <span class="switch-track"></span>
                </label>
              </div>
              <div class="setting-row memory-row">
                <div class="memory-row-main">
                  <div class="memory-row-title">允许基于工具辅助聊天生成本地记忆</div>
                  <div class="memory-row-desc">从使用过 MCP 工具或网页搜索的聊天生成记忆</div>
                </div>
                <label class="switch">
                  <input
                    type="checkbox"
                    v-model="memAllowTool"
                    :disabled="!memEnable"
                    @change="saveMemorySection()"
                  />
                  <span class="switch-track"></span>
                </label>
              </div>
              <div class="setting-row memory-row">
                <div class="memory-row-main">
                  <div class="memory-row-title">删除本地记忆</div>
                  <div class="memory-row-desc">删除存储在此电脑本地的所有记忆</div>
                </div>
                <button
                  class="btn btn-icon danger memory-delete-btn"
                  v-tooltip="'删除记忆'"
                  aria-label="删除记忆"
                  @click="resetMemory()"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_DELETE" />
                  </svg>
                </button>
              </div>
            </div>
          </div>
        </section>

        <DynamicToolsSection :active="activeSection === 'dynamic-tools'" />

        <SkillsSection :active="activeSection === 'skills'" />

        <McpSection :active="activeSection === 'mcp'" />

        <PluginsSection :active="activeSection === 'plugins'" />

      </div>
    </div>
  </div>
</template>
