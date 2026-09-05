<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  addMarketplace,
  askConfirm,
  installPlugin,
  isAuthRequiredError,
  loadMcpServerStatus,
  loadMcpServers,
  loadMemoryConfig,
  loadModelProviderConfig,
  loadPluginCatalog,
  removeMarketplace,
  saveMcpServers,
  saveMemoryConfig,
  saveModelProviderConfig,
  saveSettings,
  setToast,
  store,
  toastError,
  uninstallPlugin,
} from "../composables/useCodex";
import type {
  PluginCatalogItem,
  PluginMarketplaceInfo,
  PluginMarketplaceLoadError,
  ModelProviderConfigState,
} from "../composables/useCodex";
import {
  THEMES,
  previewTheme,
  type ThemeId,
} from "../composables/useTheme";
import { assetUrl } from "../lib/asset";
import { copyText } from "../lib/clipboard";
import { openPathInApp } from "../composables/useSessionFs";
import {
  ICON_ARROW_DOWN,
  ICON_ARROW_RIGHT,
  ICON_CHECK,
  ICON_DELETE,
  ICON_DOWNLOAD,
  ICON_EDIT,
  ICON_EXTENSION,
  ICON_FILE,
  ICON_FOLDER_OPEN,
  ICON_INFO,
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
import {
  dynamicToolRows,
  type DynamicToolRow,
} from "../lib/dynamicTools";
import { PERMISSION_MODES } from "../lib/permissions";
import type {
  AppSettings,
  CustomInstructionsState,
  McpAuthStatus,
  McpEnvEntry,
  McpServerDetail,
  McpServerInfo,
  McpToolDetail,
  ModelConfigUiEdit,
  ModelConfigState,
  ModelProviderInfo,
  SkillErrorInfo,
  SkillsItem,
  SkillsState,
  TerminalShell,
} from "../lib/types";
import ModelConfigModelPicker from "./ModelConfigModelPicker.vue";
import ModalDialog from "./ModalDialog.vue";

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
  void refreshPlugins();
  void loadModelConfig();
  void loadCustomInstructions();
  void loadSkills();
  void loadMcp();
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

// 进入「基础设置」标签时从 codex 配置回填开关；进入「插件管理」时分区变为可见，
// 补算已安装插件列表的限高（隐藏期 offsetHeight 恒为 0，测量无效）
watch(activeSection, (id) => {
  if (id === "basic") void loadMemorySection();
  if (id === "plugins") void syncInstalledListHeight();
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

/** Skills Catalog for Codex（浏览器打开） */
const SKILLS_CATALOG_URL = "https://github.com/openai/skills";

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

// ---------- AGENTS.md 自定义指令 ----------

/** CODEX_HOME/AGENTS.md 卡片状态 */
const agents = reactive({
  loading: false,
  saving: false,
  agents_path: "",
  exists: false,
  content: "",
});

async function loadCustomInstructions() {
  agents.loading = true;
  try {
    const res = await invoke<CustomInstructionsState>(
      "custom_instructions_read",
    );
    agents.agents_path = res.agents_path;
    agents.exists = res.exists;
    agents.content = res.content;
  } catch (e) {
    setToast(toastError(e));
  } finally {
    agents.loading = false;
  }
}

async function saveCustomInstructions() {
  if (agents.saving || agents.loading) return;
  agents.saving = true;
  try {
    await invoke("custom_instructions_save", { content: agents.content });
    agents.exists = true;
    setToast("AGENTS 已保存（新会话生效）");
  } catch (e) {
    setToast(toastError(e));
  } finally {
    agents.saving = false;
  }
}

function openAgentsFile() {
  void openPathInAppOrReveal(agents.agents_path);
}

// ---------- 技能管理 ----------

const skillsState = reactive({
  loading: false,
  adding: false,
  items: [] as SkillsItem[],
  errors: [] as SkillErrorInfo[],
  busy: {} as Record<string, boolean>,
});

/** 打开文件对话框选择 SKILL.md：校验通过后安装到 CODEX_HOME/skills 并刷新列表 */
async function addSkill() {
  if (skillsState.loading || skillsState.adding) return;
  skillsState.adding = true;
  try {
    const name = await invoke<string | null>("skills_add");
    if (!name) return; // 用户取消：文件未变化，不刷新
    setToast(`已添加技能 ${name}`);
    await loadSkills(true);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    skillsState.adding = false;
  }
}

/** 拉取本地技能列表（skills_read：从 skills/list 过滤出 CODEX_HOME/skills 下的技能） */
async function loadSkills(forceReload = false) {
  if (skillsState.loading) return;
  skillsState.loading = true;
  try {
    const res = await invoke<SkillsState | null>("skills_read", {
      forceReload,
    });
    skillsState.items = res?.items ?? [];
    skillsState.errors = res?.errors ?? [];
  } catch (e) {
    setToast(toastError(e));
  } finally {
    skillsState.loading = false;
  }
}

function openSkill(s: SkillsItem) {
  void openPathInAppOrReveal(s.path);
}

/** 启用/禁用技能：写用户级技能配置后强制重读列表 */
async function toggleSkill(s: SkillsItem) {
  if (skillsState.loading || skillsState.busy[s.path]) return;
  skillsState.busy[s.path] = true;
  const next = !s.enabled;
  try {
    await invoke("codex_rpc", {
      method: "skills/config/write",
      params: { name: s.name, enabled: next },
    });
    setToast(next ? `已启用 ${s.name}` : `已禁用 ${s.name}`);
    await loadSkills(true);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    skillsState.busy[s.path] = false;
  }
}

/** 删除技能：确认后删除 SKILL.md 所在目录并强制刷新列表 */
async function removeSkill(s: SkillsItem) {
  if (skillsState.loading || skillsState.busy[s.path]) return;
  const ok = await askConfirm({
    title: "删除技能",
    message: `确定删除技能「${s.name}」吗？将删除该技能所在目录（${s.path}），此操作不可恢复。`,
    confirmLabel: "删除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  skillsState.busy[s.path] = true;
  try {
    await invoke("skills_remove", { skillPath: s.path });
    setToast(`已删除技能 ${s.name}`);
    await loadSkills(true);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    skillsState.busy[s.path] = false;
  }
}

// ---------- 动态工具 ----------

/** 动态工具行（静态定义展开；当前为 codexui 命名空间两个工具），只提供启用/禁用 */
const dynamicToolRowsList = dynamicToolRows();

function isDynamicToolDisabled(key: string): boolean {
  return (store.settings.dynamic_tools_disabled ?? []).includes(key);
}

/** 切换动态工具启用/禁用：写入应用设置，禁用的工具不再注入新会话 */
async function toggleDynamicTool(tool: DynamicToolRow) {
  const disabled = new Set(store.settings.dynamic_tools_disabled ?? []);
  if (disabled.has(tool.key)) disabled.delete(tool.key);
  else disabled.add(tool.key);
  try {
    await saveSettings({ dynamic_tools_disabled: Array.from(disabled) });
  } catch (e) {
    setToast(toastError(e));
  }
}

// ---------- MCP 管理 ----------

const mcpState = reactive({
  loading: false,
  saving: false,
  servers: [] as McpServerInfo[],
  /** config/read 用户层原始 [mcp_servers.*]，用于保存时保留未知字段 */
  raw: {} as Record<string, unknown>,
});

/** omit_tools_from 可选暴露面（勾选=从该面排除），UI 展示用中文备注 */
const MCP_OMIT_OPTIONS = [
  { value: "direct", label: "直接暴露" },
  { value: "deferred", label: "延迟暴露" },
  { value: "code_mode", label: "代码模式" },
] as const;

/** MCP 新增/编辑表单状态（editingIndex < 0 表示新增） */
const mcpForm = reactive({
  open: false,
  editingIndex: -1,
  /** 传输方式："stdio" | "http" */
  transport: "stdio" as "stdio" | "http",
  name: "",
  command: "",
  cwd: "",
  argsText: "",
  env: [] as McpEnvEntry[],
  url: "",
  headers: [] as McpEnvEntry[],
  bearer_token_env_var: "",
  omit_tools_from: [] as string[],
});

const mcpFormErrors = reactive({
  name: "",
  command: "",
  url: "",
  env: "",
});

function clearMcpFormErrors() {
  mcpFormErrors.name = "";
  mcpFormErrors.command = "";
  mcpFormErrors.url = "";
  mcpFormErrors.env = "";
}

async function loadMcp() {
  if (mcpState.loading) return;
  mcpState.loading = true;
  try {
    const res = await loadMcpServers();
    mcpState.servers = res.servers.map((s) => ({ ...s }));
    mcpState.raw = res.raw;
  } catch (e) {
    setToast(toastError(e));
  } finally {
    mcpState.loading = false;
  }
}

/** 打开「添加」MCP 服务器表单 */
function openAddMcp() {
  mcpForm.open = true;
  mcpForm.editingIndex = -1;
  mcpForm.transport = "stdio";
  mcpForm.name = "";
  mcpForm.command = "";
  mcpForm.cwd = "";
  mcpForm.argsText = "";
  mcpForm.env = [];
  mcpForm.url = "";
  mcpForm.headers = [];
  mcpForm.bearer_token_env_var = "";
  mcpForm.omit_tools_from = ["deferred"];
  clearMcpFormErrors();
}

/** 打开「编辑」MCP 服务器表单（标识 name 只读） */
function openEditMcp(index: number) {
  const s = mcpState.servers[index];
  if (!s) return;
  mcpForm.open = true;
  mcpForm.editingIndex = index;
  mcpForm.name = s.name;
  mcpForm.transport = s.url.trim() ? "http" : "stdio";
  mcpForm.command = s.command;
  mcpForm.cwd = s.cwd ?? "";
  mcpForm.argsText = (s.args ?? []).join(" ");
  mcpForm.env = (s.env ?? []).map((e) => ({ ...e }));
  mcpForm.url = s.url ?? "";
  mcpForm.headers = (s.headers ?? []).map((e) => ({ ...e }));
  mcpForm.bearer_token_env_var = s.bearer_token_env_var ?? "";
  mcpForm.omit_tools_from = [...(s.omit_tools_from ?? [])];
  clearMcpFormErrors();
}

function closeMcpForm() {
  mcpForm.open = false;
}

function addMcpEnvRow() {
  mcpForm.env.push({ key: "", value: "" });
}

function removeMcpEnvRow(index: number) {
  mcpForm.env.splice(index, 1);
}

function addMcpHeaderRow() {
  mcpForm.headers.push({ key: "", value: "" });
}

function removeMcpHeaderRow(index: number) {
  mcpForm.headers.splice(index, 1);
}

/** 提交 MCP 表单：校验后更新本地列表并立即落盘 */
async function confirmMcpForm() {
  if (mcpState.saving || mcpState.loading) return;
  clearMcpFormErrors();
  const name = mcpForm.name.trim();
  if (!name) {
    mcpFormErrors.name = "请填写服务器名称";
    return;
  }
  if (!/^[A-Za-z0-9_-]+$/.test(name)) {
    mcpFormErrors.name = "只能包含字母、数字、下划线与连字符";
    return;
  }
  if (mcpForm.editingIndex < 0) {
    if (mcpState.servers.some((s) => s.name === name)) {
      mcpFormErrors.name = "服务器名称已存在";
      return;
    }
  }
  const isHttp = mcpForm.transport === "http";
  if (isHttp) {
    if (!mcpForm.url.trim()) {
      mcpFormErrors.url = "请填写 url";
      return;
    }
    if (!/^https?:\/\//i.test(mcpForm.url.trim())) {
      mcpFormErrors.url = "url 必须以 http:// 或 https:// 开头";
      return;
    }
  } else if (!mcpForm.command.trim()) {
    mcpFormErrors.command = "请填写 command";
    return;
  }
  const kvRows = isHttp ? mcpForm.headers : mcpForm.env;
  const kvLabel = isHttp ? "请求头" : "env";
  const envKeys = new Set<string>();
  for (const e of kvRows) {
    const key = e.key.trim();
    if (!key) {
      mcpFormErrors.env = `存在空的 ${kvLabel} 键`;
      return;
    }
    if (envKeys.has(key)) {
      mcpFormErrors.env = `${kvLabel}键「${key}」重复`;
      return;
    }
    envKeys.add(key);
  }
  const args = mcpForm.argsText
    .split(/\s+/)
    .map((a) => a.trim())
    .filter(Boolean);
  const entry: McpServerInfo = {
    name,
    command: isHttp ? "" : mcpForm.command.trim(),
    cwd: isHttp ? "" : mcpForm.cwd.trim(),
    args: isHttp ? [] : args,
    env: isHttp ? [] : mcpForm.env.map((e) => ({ key: e.key.trim(), value: e.value })),
    url: isHttp ? mcpForm.url.trim() : "",
    headers: isHttp
      ? mcpForm.headers.map((e) => ({ key: e.key.trim(), value: e.value }))
      : [],
    bearer_token_env_var: isHttp ? mcpForm.bearer_token_env_var.trim() : "",
    omit_tools_from: [...mcpForm.omit_tools_from],
  };
  const isNew = mcpForm.editingIndex < 0;
  if (isNew) {
    mcpState.servers.push(entry);
  } else {
    mcpState.servers[mcpForm.editingIndex] = entry;
  }
  mcpState.saving = true;
  try {
    await saveMcpServers(mcpState.servers, mcpState.raw);
    setToast("MCP 服务器已保存");
    mcpForm.open = false;
    await loadMcp();
  } catch (e) {
    setToast(toastError(e));
    // 回滚：重读磁盘状态，保留表单输入
    await loadMcp();
  } finally {
    mcpState.saving = false;
  }
}

/** 删除 MCP 服务器：弹确认后立即落盘 */
async function removeMcp(index: number) {
  const s = mcpState.servers[index];
  if (!s || mcpState.saving || mcpState.loading) return;
  const ok = await askConfirm({
    title: "删除 MCP 服务器",
    message: `确定删除 MCP 服务器「${s.name}」吗？`,
    confirmLabel: "删除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  mcpState.servers.splice(index, 1);
  mcpState.saving = true;
  try {
    await saveMcpServers(mcpState.servers, mcpState.raw);
    setToast(`已删除 MCP 服务器「${s.name}」`);
    await loadMcp();
  } catch (e) {
    setToast(toastError(e));
    await loadMcp(); // 回滚
  } finally {
    mcpState.saving = false;
  }
}

// ---------- MCP 详情信息 ----------

const MCP_DETAIL_TABS = [
  { id: "info", label: "服务器信息" },
  { id: "tools", label: "工具" },
  { id: "resources", label: "资源" },
] as const;
type McpDetailTabId = (typeof MCP_DETAIL_TABS)[number]["id"];

const MCP_AUTH_LABELS: Record<McpAuthStatus, string> = {
  unknown: "认证状态未知",
  unsupported: "未支持认证",
  notLoggedIn: "未登录",
  bearerToken: "Bearer Token",
  oAuth: "OAuth",
};

/** MCP 详情弹窗状态：不落全局 store，随设置页关闭一并销毁 */
const mcpDetail = reactive({
  open: false,
  name: "",
  transport: "" as "" | "stdio" | "http",
  command: "",
  argsText: "",
  cwd: "",
  url: "",
  loading: false,
  notFound: false,
  detail: null as McpServerDetail | null,
  startup: "" as "" | "starting" | "ready" | "failed" | "cancelled",
  activeTab: "info" as McpDetailTabId,
});

/** mcpServer/startupStatus/updated 的注销函数（弹窗打开期间仅保留一个） */
let mcpDetailUnlisten: (() => void) | null = null;

async function stopMcpDetailListen() {
  const unlisten = mcpDetailUnlisten;
  mcpDetailUnlisten = null;
  if (!unlisten) return;
  try {
    await unlisten();
  } catch {
    // 注销失败可忽略（下次打开会重新监听）
  }
}

/** 重新拉取当前详情；加载/打开失败时保持弹窗并 toast。 */
async function refreshMcpDetail() {
  if (!mcpDetail.open || mcpDetail.loading) return;
  mcpDetail.loading = true;
  mcpDetail.notFound = false;
  try {
    const detail = await loadMcpServerStatus(mcpDetail.name);
    mcpDetail.detail = detail;
    mcpDetail.notFound = detail === null;
  } catch (e) {
    setToast(toastError(e));
  } finally {
    mcpDetail.loading = false;
  }
}

/** 处理服务器启动状态通知：仅用于 ready 后自动重拉一次能力清单 */
function onMcpStartupUpdated(payload: {
  name?: string;
  status?: string;
}) {
  if (!payload || payload.name !== mcpDetail.name) return;
  const status = payload.status;
  if (
    status !== "starting" &&
    status !== "ready" &&
    status !== "failed" &&
    status !== "cancelled"
  ) {
    return;
  }
  mcpDetail.startup = status;
  if (status === "ready") {
    void refreshMcpDetail();
  }
}

/** 打开指定服务器的详情弹窗并订阅启动状态通知 */
async function openMcpDetail(index: number) {
  const s = mcpState.servers[index];
  if (!s || mcpState.loading || mcpState.saving) return;
  await stopMcpDetailListen();
  mcpDetail.open = true;
  mcpDetail.name = s.name;
  mcpDetail.transport = s.url.trim() ? "http" : "stdio";
  mcpDetail.command = s.command;
  mcpDetail.argsText = (s.args ?? []).join(" ");
  mcpDetail.cwd = s.cwd ?? "";
  mcpDetail.url = s.url;
  mcpDetail.detail = null;
  mcpDetail.notFound = false;
  mcpDetail.startup = "";
  mcpDetail.activeTab = "info";
  try {
    const unlisten = await listen<{
      name?: string;
      status?: string;
    }>("mcpServer/startupStatus/updated", (e) => {
      onMcpStartupUpdated(e.payload ?? {});
    });
    mcpDetailUnlisten = unlisten;
  } catch {
    // 通知监听失败不阻断查看
  }
  await refreshMcpDetail();
}

async function closeMcpDetail() {
  await stopMcpDetailListen();
  mcpDetail.open = false;
}

async function copyMcpDetailUri(uri: string) {
  const ok = await copyText(uri);
  setToast(ok ? "URI 已复制" : "复制失败，请手动选择复制");
}

function mcpDetailTabCount(id: McpDetailTabId): number {
  const detail = mcpDetail.detail;
  if (!detail) return 0;
  if (id === "tools") return detail.tools.length;
  if (id === "resources") {
    return detail.resources.length + detail.resourceTemplates.length;
  }
  return 0;
}

function mcpDetailSchemaText(tool: McpToolDetail): string {
  if (tool.inputSchema === undefined) return "";
  try {
    const text = JSON.stringify(tool.inputSchema, null, 2);
    return typeof text === "string" ? text : "";
  } catch {
    return "";
  }
}

function mcpDetailAuthLabel(): string {
  return mcpDetail.detail
    ? MCP_AUTH_LABELS[mcpDetail.detail.authStatus]
    : "";
}

onBeforeUnmount(() => {
  void stopMcpDetailListen();
});

// ---------- 插件管理 ----------

const pluginState = reactive({
  loading: false,
  marketplaces: [] as PluginMarketplaceInfo[],
  loadErrors: [] as PluginMarketplaceLoadError[],
  busy: {} as Record<string, boolean>,
  /** 市场折叠状态（按市场 name；默认折叠，重开设置页重置） */
  collapsed: {} as Record<string, boolean>,
  adding: false,
  source: "",
});

async function refreshPlugins(force = false) {
  pluginState.loading = true;
  try {
    const res = await loadPluginCatalog(force);
    pluginState.marketplaces = res.marketplaces;
    pluginState.loadErrors = res.marketplaceLoadErrors;
    // 插件市场默认折叠；用 ??= 保留用户本次会话内已手动展开/折叠的选择
    for (const mp of res.marketplaces) {
      pluginState.collapsed[mp.name] ??= true;
    }
  } catch (e) {
    setToast(toastError(e));
  } finally {
    pluginState.loading = false;
  }
}

async function doInstall(
  mp: PluginMarketplaceInfo,
  plugin: PluginCatalogItem,
) {
  if (pluginState.busy[plugin.id]) return;
  pluginState.busy[plugin.id] = true;
  try {
    const res = await installPlugin(mp, plugin);
    const needsAuth =
      (res.appsNeedingAuth?.length ?? 0) > 0 ||
      /needs auth|requires auth|on install/i.test(res.authPolicy ?? "");
    if (needsAuth) {
      setToast(
        `已安装 ${plugin.displayName}，但部分能力需要账号登录（当前 API key 不可用）`,
      );
    } else {
      setToast(`已安装 ${plugin.displayName}`);
    }
    await refreshPlugins();
  } catch (e) {
    if (isAuthRequiredError(e)) {
      setToast(
        `${plugin.displayName} 需要账号登录，当前 API key 不可用：${toastError(e)}`,
      );
    } else {
      setToast(toastError(e));
    }
  } finally {
    pluginState.busy[plugin.id] = false;
  }
}

async function doUninstall(plugin: PluginCatalogItem) {
  if (pluginState.busy[plugin.id]) return;
  const ok = await askConfirm({
    title: "卸载插件",
    message: `确定卸载「${plugin.displayName}」吗？`,
    confirmLabel: "卸载",
    cancelLabel: "取消",
  });
  if (!ok) return;
  pluginState.busy[plugin.id] = true;
  try {
    await uninstallPlugin(plugin.id);
    setToast(`已卸载 ${plugin.displayName}`);
    await refreshPlugins();
  } catch (e) {
    setToast(toastError(e));
  } finally {
    pluginState.busy[plugin.id] = false;
  }
}

async function doAddMarketplace() {
  const source = pluginState.source.trim();
  if (!source || pluginState.adding) return;
  pluginState.adding = true;
  try {
    await addMarketplace(source);
    pluginState.source = "";
    setToast("市场已添加");
    await refreshPlugins(true);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    pluginState.adding = false;
  }
}

async function doRemoveMarketplace(mp: PluginMarketplaceInfo) {
  const ok = await askConfirm({
    title: "移除市场",
    message: `确定移除市场「${mp.displayName}」并删除其安装根吗？`,
    confirmLabel: "移除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  try {
    await removeMarketplace(mp.name);
    setToast(`已移除 ${mp.displayName}`);
    await refreshPlugins();
  } catch (e) {
    setToast(toastError(e));
  }
}

function toggleMarketplace(mp: PluginMarketplaceInfo) {
  pluginState.collapsed[mp.name] = !pluginState.collapsed[mp.name];
}

function statusLabel(p: PluginCatalogItem): string {
  if (p.availability === "DisabledByAdmin") return "管理员已禁用";
  if (p.installed && p.enabled) return "已启用";
  if (p.installed) return "已安装（未启用）";
  return "未安装";
}

/** 已安装插件（跨市场汇总，携带来源市场用于展示；卸载只需插件 id） */
const installedPlugins = computed(() =>
  pluginState.marketplaces.flatMap((mp) =>
    mp.plugins.filter((p) => p.installed).map((plugin) => ({ mp, plugin })),
  ),
);

// 已安装列表最多显示 5 行、超出滚动：行高随描述/徽章可变，按实际行高计算容器 max-height
const INSTALLED_LIST_MAX_ROWS = 5;
const installedListEl = ref<HTMLElement | null>(null);
const installedListMaxHeight = ref("");

async function syncInstalledListHeight() {
  await nextTick();
  const rows = installedListEl.value
    ? Array.from(
        installedListEl.value.querySelectorAll<HTMLElement>(".plugin-row"),
      )
    : [];
  if (rows.length <= INSTALLED_LIST_MAX_ROWS) {
    installedListMaxHeight.value = "";
    return;
  }
  const h = rows
    .slice(0, INSTALLED_LIST_MAX_ROWS)
    .reduce((sum, row) => sum + row.offsetHeight, 0);
  // 分区处于 v-show 隐藏时 offsetHeight 全为 0，测量无效：保留现值，
  // 待切入插件管理分区（分区可见）后再由 activeSection watch 重算
  if (h <= 0) return;
  installedListMaxHeight.value = `${h}px`;
}

watch(
  () => installedPlugins.value.map((x) => x.plugin.id).join(","),
  syncInstalledListHeight,
);

function canInstall(p: PluginCatalogItem): boolean {
  return p.availability !== "DisabledByAdmin";
}

// ---------- 插件行图标（仅用 plugin/list 接口字段，远程 URL 优先，其次本地路径，最后品牌色首字母回退） ----------
const brokenPluginIcons = ref(new Set<string>());

function pluginIconSrc(p: PluginCatalogItem): string {
  if (brokenPluginIcons.value.has(p.id)) return "";
  if (p.iconUrl) return p.iconUrl;
  if (p.iconPath) return assetUrl(p.iconPath);
  return "";
}

function markIconError(id: string) {
  const s = new Set(brokenPluginIcons.value);
  s.add(id);
  brokenPluginIcons.value = s;
}

function pluginInitial(p: PluginCatalogItem): string {
  return (p.displayName.trim()[0] ?? "P").toUpperCase();
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
                <select
                  id="model-config-ui-effort"
                  v-model="modelConfig.model_reasoning_effort"
                  :disabled="modelConfig.loading"
                >
                  <option value="">默认（不写入）</option>
                  <option
                    v-for="effort in REASONING_EFFORT_VALUES"
                    :key="effort"
                    :value="effort"
                  >
                    {{ effort }}
                  </option>
                </select>
              </div>
              <div class="setting-row">
                <label for="model-config-ui-auth">认证方式</label>
                <select
                  id="model-config-ui-auth"
                  v-model="authMethod"
                  :disabled="modelConfig.loading"
                >
                  <option value="">默认（不写入）</option>
                  <option value="apikey">API Key</option>
                  <option value="chatgpt">ChatGPT 登录</option>
                </select>
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
                  <select v-model="providerForm.wire_api">
                    <option value="responses">responses</option>
                    <option value="chat">chat</option>
                  </select>
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

        <section
          v-show="activeSection === 'global-instructions'"
          class="settings-section settings-section-global-instructions"
        >
          <h2 class="settings-section-title">全局指令</h2>
          <p class="settings-section-desc">
            管理 CODEX_HOME 下的全局自定义指令（AGENTS.md）
          </p>
          <div class="model-config-card">
            <div class="model-config-card-head">
              <button
                type="button"
                class="model-config-title-link"
                v-tooltip="'在编辑器中打开文件'"
                :disabled="!agents.agents_path"
                @click="openAgentsFile"
              >
                <span>AGENTS</span>
              </button>
              <div class="model-config-head-actions">
                <span v-if="agents.agents_path && !agents.exists" class="model-config-missing">
                  （文件不存在，保存时将新建）
                </span>
                <button
                  class="btn btn-icon model-config-reload-btn"
                  aria-label="刷新"
                  v-tooltip="'刷新'"
                  :disabled="agents.loading || agents.saving"
                  @click="loadCustomInstructions"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_REFRESH" />
                  </svg>
                </button>
              </div>
            </div>
            <textarea
              v-model="agents.content"
              class="custom-instructions-textarea"
              :disabled="agents.loading"
              placeholder="在此编辑 AGENTS 内容（Codex 全局自定义指令）"
              spellcheck="false"
            ></textarea>
            <div class="model-config-actions">
              <button
                class="btn primary model-config-save-btn"
                :class="{ loading: agents.saving }"
                aria-label="保存"
                :disabled="agents.saving || agents.loading"
                @click="saveCustomInstructions"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path :d="ICON_SAVE" />
                </svg>
                <span>保存</span>
              </button>
            </div>
          </div>
        </section>

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
                <select
                  v-model="terminalShell"
                  class="terminal-shell-select"
                  @change="persist({ terminal_shell: terminalShell })"
                >
                  <option value="cmd">cmd（命令提示符）</option>
                  <option value="powershell">PowerShell</option>
                </select>
              </div>

              <div class="setting-row">
                <label>默认权限</label>
                <select
                  v-model="defaultPermission"
                  class="default-permission-select"
                  @change="persist({ default_permission: defaultPermission })"
                >
                  <option v-for="m in PERMISSION_MODES" :key="m.id" :value="m.id">
                    {{ m.label }}
                  </option>
                </select>
              </div>

              <div class="setting-row">
                <label>跟进处理方式</label>
                <select
                  v-model="followupMode"
                  @change="persist({ followup_mode: followupMode })"
                >
                  <option value="adjust">调整方向</option>
                  <option value="queue">加入队列</option>
                </select>
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

        <section
          v-show="activeSection === 'dynamic-tools'"
          class="settings-section settings-section-dynamic-tools"
        >
          <h2 class="settings-section-title">动态工具</h2>
          <p class="settings-section-desc">
            控制 codex-ui 动态工具（codexui）是否随新建会话注入；禁用的工具不再注入
          </p>
          <div class="model-config-card">
            <div class="model-config-card-head">
              <h3>对话内动态工具</h3>
              <div class="model-config-head-actions">
                <span class="dynamic-tools-hint">禁用后新会话不再注入</span>
              </div>
            </div>
            <div class="skills-list">
              <div
                v-for="tool in dynamicToolRowsList"
                :key="tool.key"
                class="dynamic-tool-row"
              >
                <div class="skill-info">
                  <button
                    type="button"
                    class="skill-row-main"
                    v-tooltip="tool.description"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_TOOL" />
                    </svg>
                    <span class="skill-name">{{ tool.display }}</span>
                  </button>
                  <p v-if="tool.description" class="skill-desc">
                    {{ tool.description }}
                  </p>
                </div>
                <div class="skill-actions">
                  <label class="switch">
                    <input
                      type="checkbox"
                      :checked="!isDynamicToolDisabled(tool.key)"
                      :aria-label="
                        isDynamicToolDisabled(tool.key) ? '启用工具' : '禁用工具'
                      "
                      @change="toggleDynamicTool(tool)"
                    />
                    <span class="switch-track"></span>
                  </label>
                </div>
              </div>
            </div>
          </div>
        </section>

        <section
          v-show="activeSection === 'skills'"
          class="settings-section settings-section-skills"
        >
          <h2 class="settings-section-title">技能管理</h2>
          <p class="settings-section-desc">
            管理 CODEX_HOME 下的本地技能（SKILL.md）
          </p>
          <div class="model-config-card">
            <div class="model-config-card-head">
              <h3>已安装技能</h3>
              <div class="model-config-head-actions">
                <button
                  type="button"
                  class="model-config-docs-link skill-catalog-link"
                  v-tooltip="'Skills Catalog for Codex（浏览器打开）'"
                  @click="openDocs(SKILLS_CATALOG_URL)"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_LINK" />
                  </svg>
                  Skills Catalog for Codex
                </button>
                <button
                  class="btn btn-icon primary skill-add-btn"
                  v-tooltip="'添加技能'"
                  aria-label="添加技能"
                  :disabled="skillsState.loading || skillsState.adding"
                  @click="addSkill"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_PLUS" />
                  </svg>
                </button>
                <button
                  class="btn btn-icon model-config-reload-btn"
                  aria-label="刷新"
                  v-tooltip="'刷新'"
                  :disabled="skillsState.loading || skillsState.adding"
                  @click="loadSkills()"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_REFRESH" />
                  </svg>
                </button>
              </div>
            </div>
            <div class="skills-list">
              <div
                v-if="skillsState.loading && !skillsState.items.length"
                class="plugin-empty"
              >
                正在加载技能…
              </div>
              <div
                v-else-if="!skillsState.items.length && skillsState.errors.length"
                class="plugin-empty"
              >
                {{ skillsState.errors.length }} 个技能因格式问题未加载
                <ul class="skill-error-list">
                  <li v-for="e in skillsState.errors" :key="e.path">
                    {{ e.message }}（{{ e.path }}）
                  </li>
                </ul>
              </div>
              <div v-else-if="!skillsState.items.length" class="plugin-empty">
                暂无可用技能
              </div>
              <div
                v-for="s in skillsState.items"
                :key="s.path"
                class="skill-row"
              >
                <div class="skill-info">
                  <button
                    type="button"
                    class="skill-row-main"
                    v-tooltip="'在编辑器中打开 SKILL.md'"
                    @click="openSkill(s)"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_FILE" />
                    </svg>
                    <span class="skill-name">{{ s.name }}</span>
                  </button>
                  <p v-if="s.description" class="skill-desc">
                    {{ s.description }}
                  </p>
                </div>
                <div class="skill-actions">
                  <label class="switch">
                    <input
                      type="checkbox"
                      :checked="s.enabled"
                      :disabled="skillsState.loading || !!skillsState.busy[s.path]"
                      :aria-label="s.enabled ? '禁用技能' : '启用技能'"
                      @change="toggleSkill(s)"
                    />
                    <span class="switch-track"></span>
                  </label>
                  <button
                    type="button"
                    class="btn btn-icon danger skill-delete-btn"
                    v-tooltip="'删除技能'"
                    aria-label="删除技能"
                    :disabled="skillsState.loading || !!skillsState.busy[s.path]"
                    @click="removeSkill(s)"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_DELETE" />
                    </svg>
                  </button>
                </div>
              </div>
            </div>
          </div>
        </section>

        <section
          v-show="activeSection === 'mcp'"
          class="settings-section settings-section-mcp"
        >
          <h2 class="settings-section-title">MCP 管理</h2>
          <p class="settings-section-desc">
            配置 MCP 服务器
          </p>
          <div class="model-config-card">
            <div class="model-config-card-head">
              <h3>MCP 服务器</h3>
              <div class="model-config-head-actions">
                <button
                  class="btn btn-icon primary mcp-config-add-btn"
                  v-tooltip="'添加'"
                  aria-label="添加"
                  :disabled="mcpState.loading || mcpState.saving"
                  @click="openAddMcp"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_PLUS" />
                  </svg>
                </button>
                <button
                  class="btn btn-icon model-config-reload-btn"
                  aria-label="刷新"
                  v-tooltip="'刷新'"
                  :disabled="mcpState.loading"
                  @click="loadMcp"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_REFRESH" />
                  </svg>
                </button>
              </div>
            </div>
            <div class="mcp-servers-list">
              <div
                v-if="mcpState.loading && !mcpState.servers.length"
                class="plugin-empty"
              >
                正在加载 MCP 服务器…
              </div>
              <div v-else-if="!mcpState.servers.length" class="plugin-empty">
                还没有 MCP 服务器，点击卡片头部「＋」创建。
              </div>
              <div
                v-for="(s, i) in mcpState.servers"
                :key="s.name"
                class="mcp-server-row"
              >
                <div class="mcp-server-info">
                  <span class="mcp-server-icon" aria-hidden="true">
                    <svg viewBox="0 0 24 24">
                      <path
                        :d="ICON_MCP"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.5"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      />
                    </svg>
                  </span>
                  <span class="mcp-server-name">{{ s.name }}</span>
                  <span class="mcp-server-type">
                    {{ s.url.trim() ? "http" : "stdio" }}
                  </span>
                  <span
                    v-if="s.omit_tools_from?.length"
                    class="mcp-server-omit"
                    v-tooltip="`omit_tools_from: ${s.omit_tools_from.join(', ')}`"
                  >
                    omit: {{ s.omit_tools_from.join("/") }}
                  </span>
                </div>
                <div class="model-provider-actions">
                  <button
                    class="btn btn-icon mcp-row-info"
                    aria-label="查看详情"
                    v-tooltip="'查看详情'"
                    :disabled="mcpState.loading || mcpState.saving"
                    @click="openMcpDetail(i)"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_INFO" />
                    </svg>
                  </button>
                  <button
                    class="btn btn-icon mcp-row-edit"
                    aria-label="编辑"
                    v-tooltip="'编辑'"
                    :disabled="mcpState.loading || mcpState.saving"
                    @click="openEditMcp(i)"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_EDIT" />
                    </svg>
                  </button>
                  <button
                    class="btn btn-icon danger mcp-row-delete"
                    aria-label="删除"
                    v-tooltip="'删除'"
                    :disabled="mcpState.loading || mcpState.saving"
                    @click="removeMcp(i)"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_DELETE" />
                    </svg>
                  </button>
                </div>
              </div>
            </div>

            <ModalDialog
              v-if="mcpForm.open"
              :title="mcpForm.editingIndex >= 0 ? '编辑 MCP 服务器' : '添加 MCP 服务器'"
              closable
              @close="closeMcpForm"
            >
              <div class="mcp-server-form">
                <div
                  class="setting-row"
                  :class="{ 'model-config-row-error': mcpFormErrors.name }"
                >
                  <label>
                    名称（name）
                    <span class="model-config-required" aria-label="必填" v-tooltip="'必填'">*</span>
                  </label>
                  <input
                    v-model="mcpForm.name"
                    type="text"
                    :disabled="mcpForm.editingIndex >= 0"
                    placeholder="如 filesystem"
                    :class="{ 'model-config-input-error': mcpFormErrors.name }"
                  />
                  <p v-if="mcpFormErrors.name" class="model-config-field-error">
                    {{ mcpFormErrors.name }}
                  </p>
                </div>
                <div class="setting-row">
                  <label for="mcp-form-transport">传输方式</label>
                  <select
                    id="mcp-form-transport"
                    v-model="mcpForm.transport"
                  >
                    <option value="stdio">stdio</option>
                    <option value="http">Streamable HTTP</option>
                  </select>
                </div>
                <div class="setting-row mcp-omit-row">
                  <label>omit_tools_from（工具暴露面）</label>
                  <div class="mcp-omit-options">
                    <p class="mcp-omit-hint">
                      勾选表示「从该暴露面排除」，该服务器工具不再以对应方式暴露给模型。
                    </p>
                    <label
                      v-for="opt in MCP_OMIT_OPTIONS"
                      :key="opt.value"
                      class="mcp-omit-option"
                    >
                      <input
                        type="checkbox"
                        :value="opt.value"
                        v-model="mcpForm.omit_tools_from"
                      />
                      <span class="mcp-omit-value">{{ opt.value }}</span>
                      <span class="mcp-omit-desc">{{ opt.label }}</span>
                    </label>
                    <p class="mcp-omit-note">
                      新增服务器默认勾选 deferred（直接内联，便于 DeepSeek 等模型使用）；若三个全勾选，工具将完全对模型隐藏。
                    </p>
                  </div>
                </div>
                <div
                  v-if="mcpForm.transport === 'stdio'"
                  class="setting-row"
                  :class="{ 'model-config-row-error': mcpFormErrors.command }"
                >
                  <label>
                    command
                    <span class="model-config-required" aria-label="必填" v-tooltip="'必填'">*</span>
                  </label>
                  <input
                    v-model="mcpForm.command"
                    type="text"
                    placeholder="如 npx"
                    :class="{ 'model-config-input-error': mcpFormErrors.command }"
                  />
                  <p
                    v-if="mcpFormErrors.command"
                    class="model-config-field-error"
                  >
                    {{ mcpFormErrors.command }}
                  </p>
                </div>
                <div
                  v-if="mcpForm.transport === 'stdio'"
                  class="setting-row"
                >
                  <label>cwd（工作目录）</label>
                  <input
                    v-model="mcpForm.cwd"
                    type="text"
                    placeholder="服务器进程启动目录，可留空（如 D:\\project）"
                  />
                </div>
                <div
                  v-if="mcpForm.transport === 'stdio'"
                  class="setting-row"
                >
                  <label>args（空格分隔）</label>
                  <input
                    class="mcp-args-input"
                    v-model="mcpForm.argsText"
                    type="text"
                    placeholder="如 -y @modelcontextprotocol/server-filesystem ."
                  />
                </div>
                <div
                  v-if="mcpForm.transport === 'stdio'"
                  class="setting-row mcp-env-block"
                  :class="{ 'model-config-row-error': mcpFormErrors.env }"
                >
                  <label>env（环境变量）</label>
                  <div class="mcp-env-rows">
                    <div
                      v-for="(e, i) in mcpForm.env"
                      :key="i"
                      class="mcp-env-row"
                    >
                      <input
                        v-model="e.key"
                        type="text"
                        placeholder="环境变量名"
                      />
                      <input
                        v-model="e.value"
                        type="text"
                        placeholder="值"
                      />
                      <button
                        class="btn btn-icon danger"
                        aria-label="删除该环境变量"
                        v-tooltip="'删除该环境变量'"
                        @click="removeMcpEnvRow(i)"
                      >
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                          <path :d="ICON_DELETE" />
                        </svg>
                      </button>
                    </div>
                  </div>
                  <p v-if="mcpFormErrors.env" class="model-config-field-error">
                    {{ mcpFormErrors.env }}
                  </p>
                </div>
                <div
                  v-if="mcpForm.transport === 'http'"
                  class="setting-row"
                  :class="{ 'model-config-row-error': mcpFormErrors.url }"
                >
                  <label>
                    url
                    <span class="model-config-required" aria-label="必填" v-tooltip="'必填'">*</span>
                  </label>
                  <input
                    v-model="mcpForm.url"
                    type="text"
                    placeholder="如 https://example.com/mcp"
                    :class="{ 'model-config-input-error': mcpFormErrors.url }"
                  />
                  <p v-if="mcpFormErrors.url" class="model-config-field-error">
                    {{ mcpFormErrors.url }}
                  </p>
                </div>
                <div
                  v-if="mcpForm.transport === 'http'"
                  class="setting-row"
                >
                  <label>bearer_token_env_var（Bearer 令牌环境变量名）</label>
                  <input
                    v-model="mcpForm.bearer_token_env_var"
                    type="text"
                    placeholder="如 MY_MCP_TOKEN"
                  />
                </div>
                <div
                  v-if="mcpForm.transport === 'http'"
                  class="setting-row mcp-env-block"
                  :class="{ 'model-config-row-error': mcpFormErrors.env }"
                >
                  <label>http_headers（静态请求头）</label>
                  <div class="mcp-env-rows">
                    <div
                      v-for="(h, i) in mcpForm.headers"
                      :key="i"
                      class="mcp-env-row"
                    >
                      <input
                        v-model="h.key"
                        type="text"
                        placeholder="请求头名"
                      />
                      <input
                        v-model="h.value"
                        type="text"
                        placeholder="值"
                      />
                      <button
                        class="btn btn-icon danger"
                        aria-label="删除该请求头"
                        v-tooltip="'删除该请求头'"
                        @click="removeMcpHeaderRow(i)"
                      >
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                          <path :d="ICON_DELETE" />
                        </svg>
                      </button>
                    </div>
                  </div>
                  <p v-if="mcpFormErrors.env" class="model-config-field-error">
                    {{ mcpFormErrors.env }}
                  </p>
                </div>
                <div class="model-config-actions">
                  <button
                    v-if="mcpForm.transport === 'stdio'"
                    class="btn btn-icon mcp-kv-add-btn"
                    aria-label="添加环境变量"
                    v-tooltip="'添加环境变量'"
                    @click="addMcpEnvRow"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_PLUS" />
                    </svg>
                  </button>
                  <button
                    v-else
                    class="btn btn-icon mcp-kv-add-btn"
                    aria-label="添加请求头"
                    v-tooltip="'添加请求头'"
                    @click="addMcpHeaderRow"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_PLUS" />
                    </svg>
                  </button>
                </div>
              </div>
              <template #foot>
                <button class="btn" @click="closeMcpForm">取消</button>
                <button
                  class="btn primary mcp-form-submit"
                  :disabled="mcpState.saving || mcpState.loading"
                  @click="confirmMcpForm"
                >
                  {{ mcpForm.editingIndex >= 0 ? "保存修改" : "添加" }}
                </button>
              </template>
            </ModalDialog>

            <ModalDialog
              v-if="mcpDetail.open"
              :title="mcpDetail.name"
              closable
              bodyClass="mcp-detail-body"
              @close="closeMcpDetail"
            >
              <div class="mcp-detail">
                <div
                  class="mcp-detail-tabs"
                  role="tablist"
                  aria-label="MCP 详情分类"
                >
                  <button
                    v-for="t in MCP_DETAIL_TABS"
                    :key="t.id"
                    type="button"
                    class="mcp-detail-tab"
                    :class="{ active: mcpDetail.activeTab === t.id }"
                    role="tab"
                    :aria-selected="mcpDetail.activeTab === t.id"
                    @click="mcpDetail.activeTab = t.id"
                  >
                    {{ t.label }}
                    <span
                      v-if="
                        (t.id === 'tools' || t.id === 'resources') &&
                        mcpDetail.detail
                      "
                      class="mcp-detail-tab-count"
                    >
                      {{ mcpDetailTabCount(t.id) }}
                    </span>
                  </button>
                </div>

                <div class="mcp-detail-body-inner">
                  <div
                    v-if="mcpDetail.loading && !mcpDetail.detail"
                    class="mcp-detail-empty"
                  >
                    正在获取服务器状态…
                  </div>
                  <div
                    v-else-if="mcpDetail.notFound"
                    class="mcp-detail-empty"
                  >
                    <p>
                      未查询到「{{ mcpDetail.name }}」的状态，服务器可能尚未被
                      codex 加载。
                    </p>
                    <button
                      class="btn"
                      :disabled="mcpDetail.loading"
                      @click="refreshMcpDetail"
                    >
                      重新查询
                    </button>
                  </div>
                  <template v-else-if="mcpDetail.detail">
                    <section
                      v-show="mcpDetail.activeTab === 'info'"
                      class="mcp-detail-pane"
                    >
                      <div class="mcp-detail-fields">
                        <div class="mcp-detail-field">
                          <span class="mcp-detail-field-label">名称</span>
                          <span class="mcp-detail-field-value mono">
                            {{ mcpDetail.detail.name }}
                          </span>
                        </div>
                        <div class="mcp-detail-field">
                          <span class="mcp-detail-field-label">传输类型</span>
                          <span class="mcp-detail-field-value">
                            {{ mcpDetail.transport === "http" ? "Streamable HTTP" : "stdio" }}
                          </span>
                        </div>
                        <template v-if="mcpDetail.transport === 'http'">
                          <div class="mcp-detail-field">
                            <span class="mcp-detail-field-label">url</span>
                            <span class="mcp-detail-field-value mono">
                              {{ mcpDetail.url }}
                            </span>
                          </div>
                        </template>
                        <template v-else>
                          <div class="mcp-detail-field">
                            <span class="mcp-detail-field-label">command</span>
                            <span class="mcp-detail-field-value mono">
                              {{ mcpDetail.command }}
                            </span>
                          </div>
                          <div
                            v-if="mcpDetail.argsText"
                            class="mcp-detail-field"
                          >
                            <span class="mcp-detail-field-label">args</span>
                            <span class="mcp-detail-field-value mono">
                              {{ mcpDetail.argsText }}
                            </span>
                          </div>
                          <div v-if="mcpDetail.cwd" class="mcp-detail-field">
                            <span class="mcp-detail-field-label">cwd</span>
                            <span class="mcp-detail-field-value mono">
                              {{ mcpDetail.cwd }}
                            </span>
                          </div>
                        </template>
                        <template
                          v-if="mcpDetail.detail.serverInfo"
                        >
                          <div
                            v-if="mcpDetail.detail.serverInfo.title"
                            class="mcp-detail-field"
                          >
                            <span class="mcp-detail-field-label">标题</span>
                            <span class="mcp-detail-field-value">
                              {{ mcpDetail.detail.serverInfo.title }}
                            </span>
                          </div>
                          <div
                            v-if="mcpDetail.detail.serverInfo.version"
                            class="mcp-detail-field"
                          >
                            <span class="mcp-detail-field-label">版本</span>
                            <span class="mcp-detail-field-value">
                              {{ mcpDetail.detail.serverInfo.version }}
                            </span>
                          </div>
                          <div
                            v-if="mcpDetail.detail.serverInfo.description"
                            class="mcp-detail-field"
                          >
                            <span class="mcp-detail-field-label">描述</span>
                            <span class="mcp-detail-field-value">
                              {{ mcpDetail.detail.serverInfo.description }}
                            </span>
                          </div>
                          <div
                            v-if="mcpDetail.detail.serverInfo.websiteUrl"
                            class="mcp-detail-field"
                          >
                            <span class="mcp-detail-field-label">网站</span>
                            <span class="mcp-detail-field-value mono">
                              {{ mcpDetail.detail.serverInfo.websiteUrl }}
                            </span>
                          </div>
                        </template>
                        <div class="mcp-detail-field">
                          <span class="mcp-detail-field-label">认证状态</span>
                          <span class="mcp-detail-field-value">
                            {{ mcpDetailAuthLabel() }}
                          </span>
                        </div>
                      </div>
                    </section>

                    <section
                      v-show="mcpDetail.activeTab === 'tools'"
                      class="mcp-detail-pane"
                    >
                      <div
                        v-if="mcpDetail.detail.tools.length === 0"
                        class="mcp-detail-empty"
                      >
                        未获取到工具（服务器可能未启动），服务器就绪后将自动更新。
                      </div>
                      <div
                        v-else
                        class="mcp-detail-tools"
                      >
                        <article
                          v-for="tool in mcpDetail.detail.tools"
                          :key="tool.name"
                          class="mcp-detail-tool"
                        >
                          <header class="mcp-detail-tool-head">
                            <span class="mcp-detail-tool-name">
                              {{ tool.name }}
                            </span>
                            <span
                              v-if="tool.title && tool.title !== tool.name"
                              class="mcp-detail-tool-title"
                            >
                              {{ tool.title }}
                            </span>
                          </header>
                          <p
                            v-if="tool.description"
                            class="mcp-detail-tool-desc"
                          >
                            {{ tool.description }}
                          </p>
                          <details
                            v-if="tool.inputSchema !== undefined"
                            class="mcp-detail-schema"
                          >
                            <summary>输入参数 JSON</summary>
                            <pre>{{ mcpDetailSchemaText(tool) }}</pre>
                          </details>
                        </article>
                      </div>
                    </section>

                    <section
                      v-show="mcpDetail.activeTab === 'resources'"
                      class="mcp-detail-pane"
                    >
                      <div
                        v-if="
                          mcpDetail.detail.resources.length === 0 &&
                          mcpDetail.detail.resourceTemplates.length === 0
                        "
                        class="mcp-detail-empty"
                      >
                        该服务器未声明资源与资源模板。
                      </div>
                      <template v-else>
                        <template
                          v-if="mcpDetail.detail.resources.length"
                        >
                          <h4 class="mcp-detail-group-title">
                            资源（{{ mcpDetail.detail.resources.length }}）
                          </h4>
                          <div class="mcp-detail-resources">
                            <div
                              v-for="r in mcpDetail.detail.resources"
                              :key="r.uri"
                              class="mcp-resource-row"
                            >
                              <div class="mcp-resource-main">
                                <span class="mcp-resource-title">
                                  {{ r.title || r.name || r.uri }}
                                </span>
                                <code class="mcp-resource-uri">
                                  {{ r.uri }}
                                </code>
                                <span
                                  v-if="r.description || r.mimeType"
                                  class="mcp-resource-meta"
                                >
                                  {{
                                    [r.description, r.mimeType]
                                      .filter(Boolean)
                                      .join(" · ")
                                  }}
                                </span>
                              </div>
                              <button
                                class="btn mcp-resource-copy-btn"
                                aria-label="复制 URI"
                                v-tooltip="'复制 URI'"
                                @click="copyMcpDetailUri(r.uri)"
                              >
                                复制
                              </button>
                            </div>
                          </div>
                        </template>
                        <template
                          v-if="mcpDetail.detail.resourceTemplates.length"
                        >
                          <h4 class="mcp-detail-group-title">
                            资源模板（{{
                              mcpDetail.detail.resourceTemplates.length
                            }}）
                          </h4>
                          <div class="mcp-detail-resources">
                            <div
                              v-for="rt in mcpDetail.detail.resourceTemplates"
                              :key="rt.uriTemplate"
                              class="mcp-resource-row"
                            >
                              <div class="mcp-resource-main">
                                <span class="mcp-resource-title">
                                  {{ rt.title || rt.name || rt.uriTemplate }}
                                </span>
                                <code class="mcp-resource-uri">
                                  {{ rt.uriTemplate }}
                                </code>
                                <span
                                  v-if="rt.description || rt.mimeType"
                                  class="mcp-resource-meta"
                                >
                                  {{
                                    [rt.description, rt.mimeType]
                                      .filter(Boolean)
                                      .join(" · ")
                                  }}
                                </span>
                              </div>
                            </div>
                          </div>
                        </template>
                      </template>
                    </section>

                  </template>
                </div>
              </div>
            </ModalDialog>

          </div>
        </section>

        <section
          v-show="activeSection === 'plugins'"
          class="settings-section settings-section-plugins"
        >
          <h2 class="settings-section-title">插件管理</h2>
          <p class="settings-section-desc">
            插件市场目录与本地安装管理
          </p>
          <div class="model-config-card">
            <div class="model-config-card-head">
              <h3>已安装插件</h3>
            </div>
            <div
              v-if="pluginState.loading && !installedPlugins.length"
              class="plugin-empty"
            >
              正在加载已安装插件…
            </div>
            <div v-else-if="!installedPlugins.length" class="plugin-empty">
              还没有已安装的插件，可在下方插件市场安装。
            </div>
            <div
              v-else
              ref="installedListEl"
              class="plugin-list installed-plugin-list"
              :style="{ maxHeight: installedListMaxHeight }"
            >
              <div
                v-for="{ mp, plugin: p } in installedPlugins"
                :key="p.id"
                class="plugin-row"
              >
                <span class="plugin-row-icon" aria-hidden="true">
                  <img
                    v-if="pluginIconSrc(p)"
                    :src="pluginIconSrc(p)"
                    alt=""
                    loading="lazy"
                    @error="markIconError(p.id)"
                  />
                  <span
                    v-else
                    class="plugin-icon-fallback"
                    :style="
                      p.brandColor ? { background: p.brandColor } : undefined
                    "
                  >
                    {{ pluginInitial(p) }}
                  </span>
                </span>
                <div class="plugin-info">
                  <div class="plugin-name">
                    {{ p.displayName }}
                    <span v-if="p.version" class="plugin-version">
                      {{ p.version }}
                    </span>
                  </div>
                  <div v-if="p.description" class="plugin-desc">
                    {{ p.description }}
                  </div>
                  <div class="plugin-meta">
                    <span class="plugin-status">{{ statusLabel(p) }}</span>
                    <span class="plugin-source">来源：{{ mp.displayName }}</span>
                    <span
                      v-if="p.disabledReason"
                      class="plugin-disabled-reason"
                    >
                      {{ p.disabledReason }}
                    </span>
                  </div>
                </div>
                <button
                  class="btn btn-icon danger plugin-uninstall-btn"
                  :class="{ loading: !!pluginState.busy[p.id] }"
                  v-tooltip="pluginState.busy[p.id] ? '卸载中…' : '卸载'"
                  :aria-label="pluginState.busy[p.id] ? '卸载中' : '卸载'"
                  :disabled="!!pluginState.busy[p.id]"
                  @click="doUninstall(p)"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_DELETE" />
                  </svg>
                </button>
              </div>
            </div>
          </div>

          <div class="model-config-card">
            <div class="model-config-card-head">
              <h3>插件市场</h3>
              <div class="model-config-head-actions">
                <button
                  class="btn btn-icon plugin-refresh-btn"
                  :class="{ loading: pluginState.loading }"
                  v-tooltip="'刷新目录'"
                  aria-label="刷新目录"
                  :disabled="pluginState.loading"
                  @click="refreshPlugins(true)"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_REFRESH" />
                  </svg>
                </button>
              </div>
            </div>
            <div class="plugin-manage-toolbar">
              <div class="plugin-market-add">
                <input
                  v-model="pluginState.source"
                  placeholder="Git URL 或本地绝对路径"
                  @keydown.enter="doAddMarketplace"
                />
                <button
                  class="btn btn-icon primary plugin-market-add-btn"
                  :class="{ loading: pluginState.adding }"
                  v-tooltip="'添加市场'"
                  aria-label="添加市场"
                  :disabled="pluginState.adding || !pluginState.source.trim()"
                  @click="doAddMarketplace"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_PLUS" />
                  </svg>
                </button>
              </div>
            </div>

            <div v-if="pluginState.loadErrors.length" class="plugin-load-errors">
              <p
                v-for="(err, i) in pluginState.loadErrors"
                :key="i"
                class="plugin-load-error"
              >
                市场「{{ err.name || "未知" }}」加载失败：{{ err.error }}
              </p>
            </div>

            <div
              v-if="pluginState.loading && !pluginState.marketplaces.length"
              class="plugin-empty"
            >
              正在加载插件目录…
            </div>
            <div v-else-if="!pluginState.marketplaces.length" class="plugin-empty">
              暂无可用市场
            </div>
            <div v-else class="plugin-marketplaces">
              <div
                v-for="mp in pluginState.marketplaces"
                :key="mp.name"
                class="plugin-marketplace"
              >
                <div
                  class="plugin-marketplace-head"
                  :class="{ collapsed: pluginState.collapsed[mp.name] }"
                  @click="toggleMarketplace(mp)"
                >
                  <span class="plugin-marketplace-arrow" aria-hidden="true">
                    <svg viewBox="0 0 24 24">
                      <path
                        :d="
                          pluginState.collapsed[mp.name]
                            ? ICON_ARROW_RIGHT
                            : ICON_ARROW_DOWN
                        "
                      />
                    </svg>
                  </span>
                  <span class="plugin-marketplace-icon" aria-hidden="true">
                    <svg viewBox="0 0 24 24">
                      <path :d="ICON_EXTENSION" />
                    </svg>
                  </span>
                  <span class="plugin-marketplace-name">{{ mp.displayName }}</span>
                  <span
                    class="plugin-marketplace-count"
                    v-tooltip="`${mp.plugins.length} 个插件`"
                  >
                    {{ mp.plugins.length }}
                  </span>
                  <button
                    v-if="!mp.isRemote"
                    class="btn btn-icon danger plugin-market-remove"
                    aria-label="移除市场"
                    v-tooltip="'移除市场'"
                    @click.stop="doRemoveMarketplace(mp)"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_DELETE" />
                    </svg>
                  </button>
                </div>
                <template v-if="!pluginState.collapsed[mp.name]">
                  <div v-if="!mp.plugins.length" class="plugin-empty small">
                    该市场暂无插件
                  </div>
                  <div v-else class="plugin-list">
                    <div v-for="p in mp.plugins" :key="p.id" class="plugin-row">
                      <span class="plugin-row-icon" aria-hidden="true">
                        <img
                          v-if="pluginIconSrc(p)"
                          :src="pluginIconSrc(p)"
                          alt=""
                          loading="lazy"
                          @error="markIconError(p.id)"
                        />
                        <span
                          v-else
                          class="plugin-icon-fallback"
                          :style="
                            p.brandColor ? { background: p.brandColor } : undefined
                          "
                        >
                          {{ pluginInitial(p) }}
                        </span>
                      </span>
                      <div class="plugin-info">
                        <div class="plugin-name">
                          {{ p.displayName }}
                          <span v-if="p.version" class="plugin-version">
                            {{ p.version }}
                          </span>
                        </div>
                        <div v-if="p.description" class="plugin-desc">
                          {{ p.description }}
                        </div>
                        <div class="plugin-meta">
                          <span class="plugin-status">{{ statusLabel(p) }}</span>
                          <span
                            v-if="p.disabledReason"
                            class="plugin-disabled-reason"
                          >
                            {{ p.disabledReason }}
                          </span>
                        </div>
                      </div>
                      <button
                        v-if="!p.installed"
                        class="btn btn-icon primary plugin-install-btn"
                        :class="{ loading: !!pluginState.busy[p.id] }"
                        v-tooltip="pluginState.busy[p.id] ? '安装中…' : '安装'"
                        :aria-label="pluginState.busy[p.id] ? '安装中' : '安装'"
                        :disabled="!canInstall(p) || !!pluginState.busy[p.id]"
                        @click="doInstall(mp, p)"
                      >
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                          <path :d="ICON_DOWNLOAD" />
                        </svg>
                      </button>
                      <button
                        v-else
                        class="btn btn-icon danger plugin-uninstall-btn"
                        :class="{ loading: !!pluginState.busy[p.id] }"
                        v-tooltip="pluginState.busy[p.id] ? '卸载中…' : '卸载'"
                        :aria-label="pluginState.busy[p.id] ? '卸载中' : '卸载'"
                        :disabled="!!pluginState.busy[p.id]"
                        @click="doUninstall(p)"
                      >
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                          <path :d="ICON_DELETE" />
                        </svg>
                      </button>
                    </div>
                  </div>
                </template>
              </div>
            </div>
          </div>
        </section>

      </div>
    </div>
  </div>
</template>
