<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  activeSessionTab,
  addMarketplace,
  askConfirm,
  installPlugin,
  isAuthRequiredError,
  loadPluginCatalog,
  removeMarketplace,
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
} from "../composables/useCodex";
import {
  THEMES,
  previewTheme,
  type ThemeId,
} from "../composables/useTheme";
import { openPathInApp } from "../composables/useSessionFs";
import {
  ICON_CHEVRON_DOWN,
  ICON_CHECK,
  ICON_CLOSE,
  ICON_DELETE,
  ICON_DOWNLOAD,
  ICON_EDIT,
  ICON_EXTENSION,
  ICON_FILE,
  ICON_FOLDER_OPEN,
  ICON_BRACES,
  ICON_MCP,
  ICON_PALETTE,
  ICON_PLUS,
  ICON_REFRESH,
  ICON_RESTART,
  ICON_SAVE,
  ICON_SKILL,
  ICON_TUNE,
} from "../lib/icons";
import { PERMISSION_MODES } from "../lib/permissions";
import type {
  AppSettings,
  CustomInstructionsState,
  McpEnvEntry,
  McpServerInfo,
  McpServersEdit,
  McpServersState,
  ModelConfigUiEdit,
  ModelConfigState,
  ModelProviderInfo,
  SkillsItem,
  SkillsState,
  TerminalShell,
} from "../lib/types";

const codexPath = ref(store.settings.codex_path ?? "");
const sound = ref(store.settings.sound_enabled);
const enterToSend = ref(store.settings.enter_to_send);
const followupMode = ref(store.settings.followup_mode);
const theme = ref<ThemeId>(store.settings.theme as ThemeId);
const defaultPermission = ref(store.settings.default_permission);
const memoryMode = ref(store.settings.memory_mode);
const terminalShell = ref<TerminalShell>(store.settings.terminal_shell);
/** 设置分类（左侧纵向导航；后续新增大类只需在此追加并补充右侧内容区） */
const settingsSections = [
  { id: "personalization", label: "个性化", icon: ICON_PALETTE },
  { id: "general", label: "通用设置", icon: ICON_TUNE },
  { id: "model-config", label: "模型配置", icon: ICON_BRACES },
  { id: "skills", label: "技能管理", icon: ICON_SKILL },
  { id: "mcp", label: "MCP管理", icon: ICON_MCP },
  { id: "plugins", label: "插件管理", icon: ICON_EXTENSION },
] as const;
type SettingsSectionId = (typeof settingsSections)[number]["id"];
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

/** 记忆模式变更：立即保存并同步当前会话（失败 toast 不阻塞） */
async function onMemoryModeChange() {
  await persist({ memory_mode: memoryMode.value });
  const threadId = activeSessionTab()?.threadId;
  if (!threadId) return;
  try {
    await invoke("codex_rpc", {
      method: "thread/memoryMode/set",
      params: { threadId, mode: memoryMode.value },
    });
  } catch (e) {
    setToast(toastError(e));
  }
}

async function resetMemory() {
  const ok = await askConfirm({
    title: "重置记忆",
    message: "将清空全部已保存的记忆，且无法撤销。是否继续？",
    confirmLabel: "重置记忆",
    cancelLabel: "取消",
  });
  if (!ok) return;
  try {
    await invoke("codex_rpc", { method: "memory/reset", params: null });
    setToast("记忆已重置");
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

/** config / model_catalog_json 卡片状态（字段与 Rust 端 model_config_read 返回一致） */
const modelConfig = reactive({
  loading: false,
  savingCatalog: false,
  savingProviders: false,
  config_path: "",
  config_exists: false,
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
  providers: [] as ModelProviderInfo[],
});

/** 模型提供方卡片校验状态：空串表示无错误；catalogWarning 为黄色警告（不阻断保存） */
const modelConfigErrors = reactive({
  model: "",
  provider: "",
  catalogWarning: "",
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
function applyProvidersCard(res: ModelConfigState) {
  modelConfig.model = res.model;
  modelConfig.model_reasoning_effort = res.model_reasoning_effort;
  modelConfig.model_provider = res.model_provider;
  modelConfig.preferred_auth_method = res.preferred_auth_method;
  modelConfig.forced_login_method = res.forced_login_method;
  modelConfig.model_catalog_json = res.model_catalog_json;
  modelConfig.openai_api_key_present = res.openai_api_key_present;
  // 浅拷贝：组件内增删改不污染调用方数组引用（测试/热更新下尤其重要）
  modelConfig.providers = (res.providers ?? []).map((p) => ({ ...p }));
  // 由 preferred_auth_method/forced_login_method 推导「认证方式」下拉
  if (
    res.preferred_auth_method === "apikey" &&
    res.forced_login_method === "api"
  ) {
    authMethod.value = "apikey";
  } else if (
    res.preferred_auth_method === "chatgpt" &&
    res.forced_login_method === "chatgpt"
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
    applyProvidersCard(res);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    modelConfig.loading = false;
  }
}

/** config 卡片刷新：重新从磁盘读取并只应用该卡片字段 */
async function refreshModelConfig() {
  modelConfig.loading = true;
  try {
    const res = await invoke<ModelConfigState>("model_config_read");
    applyConfigCard(res);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    modelConfig.loading = false;
  }
}

/** model_catalog_json 卡片刷新：重新从磁盘读取并只应用该卡片字段 */
async function refreshCatalog() {
  modelConfig.loading = true;
  try {
    const res = await invoke<ModelConfigState>("model_config_read");
    applyCatalogCard(res);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    modelConfig.loading = false;
  }
}

/** 模型提供方卡片刷新：重新从磁盘读取并只应用该卡片字段 */
async function refreshProviders() {
  modelConfig.loading = true;
  try {
    const res = await invoke<ModelConfigState>("model_config_read");
    applyProvidersCard(res);
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

/** 保存可视化模型配置：整状态同步，其余 TOML 内容由后端保留 */
async function saveProviders() {
  if (modelConfig.savingProviders || modelConfig.loading) return;
  modelConfigErrors.model = "";
  modelConfigErrors.provider = "";
  modelConfigErrors.catalogWarning = "";
  let blocked = false;
  if (!modelConfig.model.trim()) {
    modelConfigErrors.model = "请填写 model（模型名称）";
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

  // model_catalog_json 目标文件存在性检查：不存在给出黄色警告，但允许保存
  let catalogExists = true;
  const catalogValue = modelConfig.model_catalog_json.trim();
  if (catalogValue) {
    try {
      catalogExists = await invoke<boolean>("model_catalog_target_exists", {
        value: catalogValue,
      });
    } catch (e) {
      setToast(toastError(e));
    }
  }
  if (!catalogExists) {
    modelConfigErrors.catalogWarning =
      "model_catalog_json 目标文件不存在，保存后将在读取时自动创建（空模型目录）";
  }

  modelConfig.savingProviders = true;
  try {
    const auth = authMethodToEdit();
    const input: ModelConfigUiEdit = {
      model: modelConfig.model.trim(),
      model_reasoning_effort: modelConfig.model_reasoning_effort.trim(),
      model_provider: modelConfig.model_provider,
      preferred_auth_method: auth.preferred_auth_method,
      forced_login_method: auth.forced_login_method,
      model_catalog_json: catalogValue,
      providers: modelConfig.providers,
    };
    await invoke("model_config_ui_save", { input });
    setToast("模型配置已保存（重启应用后生效）");
    if (!catalogExists) {
      modelConfigErrors.catalogWarning =
        "已保存；model_catalog_json 目标文件将在读取时自动创建";
    }
    // config 内容已变化：重读提供方、原始 config 与 model_catalog_json 卡片
    await refreshProviders();
    await refreshModelConfig();
    await refreshCatalog();
  } catch (e) {
    setToast(toastError(e));
  } finally {
    modelConfig.savingProviders = false;
  }
}

async function saveCatalog() {
  if (modelConfig.savingCatalog || modelConfig.loading) return;
  modelConfig.savingCatalog = true;
  try {
    await invoke("model_catalog_save", { content: modelConfig.model_catalog });
    modelConfig.model_catalog_exists = true;
    setToast("model_catalog_json 已保存");
  } catch (e) {
    setToast(toastError(e));
  } finally {
    modelConfig.savingCatalog = false;
  }
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

function openModelConfigFile() {
  void openPathInAppOrReveal(modelConfig.config_path);
}

function openCatalogFile() {
  void openPathInAppOrReveal(modelConfig.model_catalog_path);
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
  items: [] as SkillsItem[],
});

/** 拉取本地技能列表（skills_read：CODEX_HOME/skills 文件夹） */
async function loadSkills() {
  if (skillsState.loading) return;
  skillsState.loading = true;
  try {
    const res = await invoke<SkillsState | null>("skills_read");
    skillsState.items = res?.items ?? [];
  } catch (e) {
    setToast(toastError(e));
  } finally {
    skillsState.loading = false;
  }
}

function openSkill(s: SkillsItem) {
  void openPathInAppOrReveal(s.path);
}

// ---------- MCP 管理 ----------

const mcpState = reactive({
  loading: false,
  saving: false,
  config_path: "",
  servers: [] as McpServerInfo[],
});

/** MCP 新增/编辑表单状态（editingIndex < 0 表示新增） */
const mcpForm = reactive({
  open: false,
  editingIndex: -1,
  /** 传输方式："stdio" | "http" */
  transport: "stdio" as "stdio" | "http",
  name: "",
  command: "",
  argsText: "",
  env: [] as McpEnvEntry[],
  url: "",
  headers: [] as McpEnvEntry[],
  bearer_token_env_var: "",
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
    const res = await invoke<McpServersState | null>("mcp_servers_read");
    if (res) {
      mcpState.config_path = res.config_path;
      mcpState.servers = (res.servers ?? []).map((s) => ({ ...s }));
    }
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
  mcpForm.argsText = "";
  mcpForm.env = [];
  mcpForm.url = "";
  mcpForm.headers = [];
  mcpForm.bearer_token_env_var = "";
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
  mcpForm.argsText = (s.args ?? []).join(" ");
  mcpForm.env = (s.env ?? []).map((e) => ({ ...e }));
  mcpForm.url = s.url ?? "";
  mcpForm.headers = (s.headers ?? []).map((e) => ({ ...e }));
  mcpForm.bearer_token_env_var = s.bearer_token_env_var ?? "";
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
    args: isHttp ? [] : args,
    env: isHttp ? [] : mcpForm.env.map((e) => ({ key: e.key.trim(), value: e.value })),
    url: isHttp ? mcpForm.url.trim() : "",
    headers: isHttp
      ? mcpForm.headers.map((e) => ({ key: e.key.trim(), value: e.value }))
      : [],
    bearer_token_env_var: isHttp ? mcpForm.bearer_token_env_var.trim() : "",
  };
  const isNew = mcpForm.editingIndex < 0;
  if (isNew) {
    mcpState.servers.push(entry);
  } else {
    mcpState.servers[mcpForm.editingIndex] = entry;
  }
  mcpState.saving = true;
  try {
    const input: McpServersEdit = { servers: mcpState.servers };
    await invoke("mcp_servers_save", { input });
    setToast("MCP 服务器已保存（重启应用后生效）");
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
    message: `确定删除 MCP 服务器「${s.name}」吗？删除后立即写入 config.toml。`,
    confirmLabel: "删除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  mcpState.servers.splice(index, 1);
  mcpState.saving = true;
  try {
    const input: McpServersEdit = { servers: mcpState.servers };
    await invoke("mcp_servers_save", { input });
    setToast(`已删除 MCP 服务器「${s.name}」（重启应用后生效）`);
    await loadMcp();
  } catch (e) {
    setToast(toastError(e));
    await loadMcp(); // 回滚
  } finally {
    mcpState.saving = false;
  }
}

// ---------- 插件管理 ----------

const pluginState = reactive({
  loading: false,
  marketplaces: [] as PluginMarketplaceInfo[],
  loadErrors: [] as PluginMarketplaceLoadError[],
  busy: {} as Record<string, boolean>,
  /** 市场折叠状态（按市场 name；默认展开，重开设置页重置） */
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

function canInstall(p: PluginCatalogItem): boolean {
  return p.availability !== "DisabledByAdmin";
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
            <path :d="s.icon" />
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
            主题外观、音效与消息发送偏好
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
                <label>主题外观</label>
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
            模型、模型提供方与全局自定义指令
          </p>

          <div class="model-config-card">
            <div class="model-config-card-head">
              <h3>模型提供方</h3>
              <div class="model-config-head-actions">
                <button
                  class="btn-icon model-config-reload-btn"
                  title="重读"
                  :disabled="modelConfig.loading"
                  @click="refreshProviders"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_REFRESH" />
                  </svg>
                </button>
                <div class="model-config-path">
                  <button
                    v-if="modelConfig.config_path"
                    type="button"
                    class="model-config-path-link"
                    title="在编辑器中打开 config.toml"
                    @click="openModelConfigFile"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_FILE" />
                    </svg>
                    {{ modelConfig.config_path }}
                  </button>
                  <template v-else>正在读取路径…</template>
                </div>
              </div>
            </div>

            <div class="settings">
              <div
                class="setting-row"
                :class="{ 'model-config-row-error': modelConfigErrors.model }"
              >
                <label for="model-config-ui-model">
                  model（模型名称）
                  <span class="model-config-required" title="必填">*</span>
                </label>
                <input
                  id="model-config-ui-model"
                  v-model="modelConfig.model"
                  type="text"
                  :disabled="modelConfig.loading"
                  placeholder="如 deepseek-v4-flash"
                  :class="{ 'model-config-input-error': modelConfigErrors.model }"
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
                  <option value="low">low</option>
                  <option value="high">high</option>
                  <option value="max">max</option>
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
              <div class="setting-row">
                <label for="model-config-ui-catalog">model_catalog_json</label>
                <input
                  id="model-config-ui-catalog"
                  v-model="modelConfig.model_catalog_json"
                  type="text"
                  :disabled="modelConfig.loading"
                  placeholder="如 models.json 或绝对路径"
                />
              </div>
            </div>

            <div class="model-providers-list">
              <div v-if="modelConfig.providers.length === 0" class="model-providers-empty">
                还没有提供方，点击下方「添加」创建。
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
                    <span class="model-config-required" title="必填">*</span>
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
                    title="编辑"
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
                    :title="
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

            <div v-if="providerForm.open" class="model-provider-form">
              <div
                class="setting-row"
                :class="{ 'model-config-row-error': providerFormErrors.key }"
              >
                <label>
                  标识（key）
                  <span class="model-config-required" title="必填">*</span>
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
                  <span class="model-config-required" title="必填">*</span>
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
                  <span class="model-config-required" title="必填">*</span>
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
              <div class="model-config-actions">
                <button
                  class="btn btn-icon primary provider-form-submit"
                  :title="
                    providerForm.editingIndex >= 0 ? '保存修改' : '添加'
                  "
                  @click="confirmProviderForm"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_CHECK" />
                  </svg>
                </button>
                <button
                  class="btn btn-icon provider-form-cancel"
                  title="取消"
                  @click="closeProviderForm"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_CLOSE" />
                  </svg>
                </button>
              </div>
            </div>

            <p
              v-if="modelConfigErrors.catalogWarning"
              class="model-config-warning"
            >
              {{ modelConfigErrors.catalogWarning }}
            </p>

            <div class="model-config-actions">
              <button
                class="btn primary"
                :disabled="modelConfig.savingProviders || modelConfig.loading"
                @click="saveProviders"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path :d="ICON_SAVE" />
                </svg>
                {{ modelConfig.savingProviders ? "保存中…" : "保存" }}
              </button>
              <button
                class="btn model-config-add-btn"
                :disabled="modelConfig.loading"
                @click="openAddProvider"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path :d="ICON_PLUS" />
                </svg>
                添加
              </button>
            </div>
          </div>

          <div class="model-config-card">
            <div class="model-config-card-head">
              <h3>model_catalog_json</h3>
              <div class="model-config-head-actions">
                <button
                  class="btn-icon model-config-reload-btn"
                  title="重读"
                  :disabled="modelConfig.loading"
                  @click="refreshCatalog"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_REFRESH" />
                  </svg>
                </button>
                <div class="model-config-path">
                  <template v-if="modelConfig.model_catalog_path">
                    <button
                      type="button"
                      class="model-config-path-link"
                      title="在编辑器中打开文件"
                      @click="openCatalogFile"
                    >
                      <svg viewBox="0 0 24 24" aria-hidden="true">
                        <path :d="ICON_FILE" />
                      </svg>
                      {{ modelConfig.model_catalog_path }}
                    </button>
                    <span v-if="!modelConfig.model_catalog_exists" class="model-config-missing">
                      （文件不存在，无法编辑）
                    </span>
                  </template>
                  <template v-else>
                    {{ modelConfig.config_path ? "（config 未配置 model_catalog_json）" : "正在读取路径…" }}
                  </template>
                </div>
              </div>
            </div>
            <textarea
              v-model="modelConfig.model_catalog"
              class="model-config-textarea"
              :disabled="modelConfig.loading || !modelConfig.model_catalog_exists"
              placeholder="在此编辑 model_catalog_json 目标文件内容（必须是合法 JSON）"
              spellcheck="false"
            ></textarea>
            <div class="model-config-actions">
              <button
                class="btn primary"
                :disabled="
                  modelConfig.savingCatalog ||
                  modelConfig.loading ||
                  !modelConfig.model_catalog_exists
                "
                @click="saveCatalog"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path :d="ICON_SAVE" />
                </svg>
                {{ modelConfig.savingCatalog ? "保存中…" : "保存" }}
              </button>
            </div>
          </div>

          <div class="model-config-card">
            <div class="model-config-card-head">
              <h3>AGENTS</h3>
              <div class="model-config-head-actions">
                <button
                  class="btn-icon model-config-reload-btn"
                  title="重读"
                  :disabled="agents.loading || agents.saving"
                  @click="loadCustomInstructions"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_REFRESH" />
                  </svg>
                </button>
                <div class="model-config-path">
                  <button
                    v-if="agents.agents_path"
                    type="button"
                    class="model-config-path-link"
                    title="在编辑器中打开文件"
                    @click="openAgentsFile"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_FILE" />
                    </svg>
                    {{ agents.agents_path }}
                  </button>
                  <template v-else>正在读取路径…</template>
                  <span v-if="agents.agents_path && !agents.exists" class="model-config-missing">
                    （文件不存在，保存时将新建）
                  </span>
                </div>
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
                class="btn primary"
                :disabled="agents.saving || agents.loading"
                @click="saveCustomInstructions"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path :d="ICON_SAVE" />
                </svg>
                {{ agents.saving ? "保存中…" : "保存" }}
              </button>
            </div>
          </div>
        </section>

        <section
          v-show="activeSection === 'general'"
          class="settings-section settings-section-general"
        >
          <h2 class="settings-section-title">通用设置</h2>
          <p class="settings-section-desc">
            终端、权限与记忆等基础行为
          </p>
          <div class="settings-card">
            <div class="settings">
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
                <label>跟进处理方式</label>
                <select
                  v-model="followupMode"
                  @change="persist({ followup_mode: followupMode })"
                >
                  <option value="adjust">调整方向</option>
                  <option value="queue">加入队列</option>
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
                <label>记忆模式</label>
                <div class="setting-path-row">
                  <select
                    v-model="memoryMode"
                    class="memory-mode-select"
                    @change="onMemoryModeChange"
                  >
                    <option value="disabled">关闭</option>
                    <option value="enabled">启用</option>
                  </select>
                  <button class="btn danger memory-reset-btn" @click="resetMemory()">
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_RESTART" />
                    </svg>
                    重置记忆
                  </button>
                </div>
              </div>

              <div class="setting-row">
                <label>codex 可执行文件（留空自动查找）</label>
                <div class="setting-path-row codex-path-row">
                  <div class="setting-value codex-path-value">
                    {{ codexPath || "未设置（自动查找）" }}
                  </div>
                  <button class="btn codex-pick-btn" @click="pickCodexFile()">
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_FOLDER_OPEN" />
                    </svg>
                    选择文件
                  </button>
                  <button
                    v-if="codexPath"
                    class="btn danger codex-clear-btn"
                    @click="clearCodexPath()"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_DELETE" />
                    </svg>
                    清除
                  </button>
                </div>
                <p v-if="!codexPath && store.server.codexPath" class="setting-note">
                  当前使用（自动检测）：{{ store.server.codexPath }}
                </p>
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
                  class="btn-icon model-config-reload-btn"
                  title="刷新"
                  :disabled="skillsState.loading"
                  @click="loadSkills"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_REFRESH" />
                  </svg>
                </button>
                <div class="model-config-path">
                  共 {{ skillsState.items.length }} 个
                </div>
              </div>
            </div>
            <div class="skills-list">
              <div
                v-if="skillsState.loading && !skillsState.items.length"
                class="plugin-empty"
              >
                正在加载技能…
              </div>
              <div v-else-if="!skillsState.items.length" class="plugin-empty">
                暂无可用技能
              </div>
              <div
                v-for="s in skillsState.items"
                :key="s.path"
                class="skill-row"
              >
                <button
                  type="button"
                  class="skill-row-main"
                  title="在编辑器中打开 SKILL.md"
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
                <div class="skill-path">{{ s.path }}</div>
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
            配置 MCP 服务器（写入 config.toml，重启后生效）
          </p>
          <div class="model-config-card">
            <div class="model-config-card-head">
              <h3>MCP 服务器</h3>
              <div class="model-config-head-actions">
                <button
                  class="btn mcp-config-add-btn"
                  :disabled="mcpState.loading || mcpState.saving"
                  @click="openAddMcp"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_PLUS" />
                  </svg>
                  添加
                </button>
                <button
                  class="btn-icon model-config-reload-btn"
                  title="重读"
                  :disabled="mcpState.loading"
                  @click="loadMcp"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_REFRESH" />
                  </svg>
                </button>
                <div class="model-config-path">
                  {{ mcpState.config_path || "正在读取路径…" }}
                </div>
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
                还没有 MCP 服务器，点击下方「添加」创建。
              </div>
              <div
                v-for="(s, i) in mcpState.servers"
                :key="s.name"
                class="mcp-server-row"
              >
                <div class="mcp-server-info">
                  <span class="mcp-server-name">{{ s.name }}</span>
                  <span class="mcp-server-type">
                    {{ s.url.trim() ? "http" : "stdio" }}
                  </span>
                </div>
                <div class="model-provider-actions">
                  <button
                    class="btn btn-icon mcp-row-edit"
                    title="编辑"
                    :disabled="mcpState.loading || mcpState.saving"
                    @click="openEditMcp(i)"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="ICON_EDIT" />
                    </svg>
                  </button>
                  <button
                    class="btn btn-icon danger mcp-row-delete"
                    title="删除"
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

            <div v-if="mcpForm.open" class="mcp-server-form">
              <div
                class="setting-row"
                :class="{ 'model-config-row-error': mcpFormErrors.name }"
              >
                <label>
                  名称（name）
                  <span class="model-config-required" title="必填">*</span>
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
              <div
                v-if="mcpForm.transport === 'stdio'"
                class="setting-row"
                :class="{ 'model-config-row-error': mcpFormErrors.command }"
              >
                <label>
                  command
                  <span class="model-config-required" title="必填">*</span>
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
                      title="删除该环境变量"
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
                  <span class="model-config-required" title="必填">*</span>
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
                      title="删除该请求头"
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
                  title="添加环境变量"
                  @click="addMcpEnvRow"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_PLUS" />
                  </svg>
                </button>
                <button
                  v-else
                  class="btn btn-icon mcp-kv-add-btn"
                  title="添加请求头"
                  @click="addMcpHeaderRow"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_PLUS" />
                  </svg>
                </button>
                <button
                  class="btn btn-icon primary mcp-form-submit"
                  :title="mcpForm.editingIndex >= 0 ? '保存修改' : '添加'"
                  :disabled="mcpState.saving || mcpState.loading"
                  @click="confirmMcpForm"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_CHECK" />
                  </svg>
                </button>
                <button
                  class="btn btn-icon mcp-form-cancel"
                  title="取消"
                  @click="closeMcpForm"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_CLOSE" />
                  </svg>
                </button>
              </div>
            </div>

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
              <h3>插件市场</h3>
              <div class="model-config-head-actions">
                <button
                  class="btn plugin-refresh-btn"
                  :disabled="pluginState.loading"
                  @click="refreshPlugins(true)"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_REFRESH" />
                  </svg>
                  {{ pluginState.loading ? "刷新中…" : "刷新目录" }}
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
                  class="btn"
                  :disabled="pluginState.adding || !pluginState.source.trim()"
                  @click="doAddMarketplace"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_PLUS" />
                  </svg>
                  {{ pluginState.adding ? "添加中…" : "添加市场" }}
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
                  <span class="plugin-marketplace-chevron" aria-hidden="true">
                    <svg viewBox="0 0 24 24">
                      <path :d="ICON_CHEVRON_DOWN" />
                    </svg>
                  </span>
                  <span class="plugin-marketplace-name">{{ mp.displayName }}</span>
                  <span class="plugin-marketplace-kind">
                    {{ mp.isRemote ? "官方远程目录" : "本地市场" }}
                  </span>
                  <button
                    v-if="!mp.isRemote"
                    class="btn btn-icon danger plugin-market-remove"
                    title="移除市场"
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
                        class="btn primary plugin-install-btn"
                        :disabled="!canInstall(p) || !!pluginState.busy[p.id]"
                        @click="doInstall(mp, p)"
                      >
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                          <path :d="ICON_DOWNLOAD" />
                        </svg>
                        {{ pluginState.busy[p.id] ? "安装中…" : "安装" }}
                      </button>
                      <button
                        v-else
                        class="btn danger plugin-uninstall-btn"
                        :disabled="!!pluginState.busy[p.id]"
                        @click="doUninstall(p)"
                      >
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                          <path :d="ICON_DELETE" />
                        </svg>
                        {{ pluginState.busy[p.id] ? "卸载中…" : "卸载" }}
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
