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
import { ICON_CHEVRON_DOWN, ICON_REFRESH } from "../lib/icons";
import { PERMISSION_MODES } from "../lib/permissions";
import type {
  AppSettings,
  CustomInstructionsState,
  ModelConfigState,
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
  { id: "personalization", label: "个性化" },
  { id: "model-config", label: "模型配置" },
  { id: "general", label: "通用设置" },
  { id: "plugins", label: "插件管理" },
] as const;
type SettingsSectionId = (typeof settingsSections)[number]["id"];
/** 当前选中分类：默认取第一个分类（不依赖具体标签）；设置标签存在期间保持状态，关闭后重开才重置 */
const activeSection = ref<SettingsSectionId>(settingsSections[0].id);

onMounted(() => {
  void refreshPlugins();
  void loadModelConfig();
  void loadCustomInstructions();
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

/** config.toml / models.json 表单状态（字段与 Rust 端 model_config_read 返回一致） */
const modelConfig = reactive({
  loading: false,
  savingConfig: false,
  savingModelsJson: false,
  config_path: "",
  config_exists: false,
  model: "",
  model_reasoning_effort: "",
  name: "",
  base_url: "",
  experimental_bearer_token: "",
  models_json_path: "",
  models_json_exists: false,
  models_json: "",
});

/** 应用 config.toml 卡片字段（刷新该卡片时不影响 models.json 文本框） */
function applyConfigCard(res: ModelConfigState) {
  modelConfig.config_path = res.config_path;
  modelConfig.config_exists = res.config_exists;
  modelConfig.model = res.model;
  modelConfig.model_reasoning_effort = res.model_reasoning_effort;
  modelConfig.name = res.name;
  modelConfig.base_url = res.base_url;
  modelConfig.experimental_bearer_token = res.experimental_bearer_token;
}

/** 应用 models.json 卡片字段 */
function applyModelsJsonCard(res: ModelConfigState) {
  modelConfig.models_json_path = res.models_json_path;
  modelConfig.models_json_exists = res.models_json_exists;
  modelConfig.models_json = res.models_json;
}

async function loadModelConfig() {
  modelConfig.loading = true;
  try {
    const res = await invoke<ModelConfigState>("model_config_read");
    applyConfigCard(res);
    applyModelsJsonCard(res);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    modelConfig.loading = false;
  }
}

/** config.toml 卡片刷新：重新从磁盘读取并只应用该卡片字段 */
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

/** models.json 卡片刷新：重新从磁盘读取并只应用该卡片字段 */
async function refreshModelsJson() {
  modelConfig.loading = true;
  try {
    const res = await invoke<ModelConfigState>("model_config_read");
    applyModelsJsonCard(res);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    modelConfig.loading = false;
  }
}

async function saveModelConfig() {
  if (modelConfig.savingConfig || modelConfig.loading) return;
  modelConfig.savingConfig = true;
  try {
    await invoke("model_config_save", {
      input: {
        model: modelConfig.model,
        model_reasoning_effort: modelConfig.model_reasoning_effort,
        name: modelConfig.name,
        base_url: modelConfig.base_url,
        experimental_bearer_token: modelConfig.experimental_bearer_token,
      },
    });
    modelConfig.config_exists = true;
    setToast("config.toml 已保存（重启应用后生效）");
  } catch (e) {
    setToast(toastError(e));
  } finally {
    modelConfig.savingConfig = false;
  }
}

async function saveModelsJson() {
  if (modelConfig.savingModelsJson || modelConfig.loading) return;
  modelConfig.savingModelsJson = true;
  try {
    await invoke("models_json_save", { content: modelConfig.models_json });
    modelConfig.models_json_exists = true;
    setToast("models.json 已保存");
  } catch (e) {
    setToast(toastError(e));
  } finally {
    modelConfig.savingModelsJson = false;
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

function openModelsJsonFile() {
  void openPathInAppOrReveal(modelConfig.models_json_path);
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
    setToast("AGENTS.md 已保存（新会话生效）");
  } catch (e) {
    setToast(toastError(e));
  } finally {
    agents.saving = false;
  }
}

function openAgentsFile() {
  void openPathInAppOrReveal(agents.agents_path);
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
      <nav class="settings-nav" aria-label="设置分类">
        <button
          v-for="s in settingsSections"
          :key="s.id"
          class="settings-nav-item"
          :class="{ active: activeSection === s.id }"
          :aria-pressed="activeSection === s.id"
          @click="activeSection = s.id"
        >
          {{ s.label }}
        </button>
      </nav>
      <div class="settings-panel">
        <section
          v-show="activeSection === 'personalization'"
          class="settings-section settings-section-personalization"
        >
          <h2 class="settings-section-title">个性化</h2>
          <div class="settings">
            <div class="setting-row checkbox-row">
              <input
                id="sound"
                v-model="sound"
                type="checkbox"
                @change="persist({ sound_enabled: sound })"
              />
              <label for="sound" style="margin: 0">提权/交互时播放提示音</label>
            </div>

            <div class="setting-row checkbox-row">
              <input
                id="enter"
                v-model="enterToSend"
                type="checkbox"
                @change="persist({ enter_to_send: enterToSend })"
              />
              <label for="enter" style="margin: 0">
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
                </button>
              </div>
            </div>
          </div>
        </section>

        <section
          v-show="activeSection === 'model-config'"
          class="settings-section settings-section-model-config"
        >
          <h2 class="settings-section-title">模型配置</h2>

          <div class="model-config-card">
            <div class="model-config-card-head">
              <h3>config.toml</h3>
              <div class="model-config-head-right">
                <button
                  v-if="modelConfig.config_path && modelConfig.config_exists"
                  type="button"
                  class="model-config-path-link"
                  title="在编辑器中打开文件"
                  @click="openModelConfigFile"
                >
                  {{ modelConfig.config_path }}
                </button>
                <div v-else class="model-config-path">
                  {{ modelConfig.config_path || "正在读取路径…" }}
                  <span v-if="modelConfig.config_path && !modelConfig.config_exists" class="model-config-missing">
                    （文件不存在，保存时将新建）
                  </span>
                </div>
                <button
                  class="model-config-refresh-btn"
                  type="button"
                  title="重新从磁盘读取"
                  :disabled="modelConfig.loading"
                  @click="refreshModelConfig"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_REFRESH" />
                  </svg>
                </button>
              </div>
            </div>

            <div class="settings">
              <div class="setting-row">
                <label for="model-config-model">model</label>
                <input
                  id="model-config-model"
                  v-model="modelConfig.model"
                  type="text"
                  :disabled="modelConfig.loading"
                  placeholder="如 deepseek-v4-flash"
                />
              </div>
              <div class="setting-row">
                <label for="model-config-effort">model_reasoning_effort</label>
                <input
                  id="model-config-effort"
                  v-model="modelConfig.model_reasoning_effort"
                  type="text"
                  :disabled="modelConfig.loading"
                  placeholder="low / high / max"
                />
              </div>
              <div class="setting-row">
                <label for="model-config-provider-name">name（模型提供方名称）</label>
                <input
                  id="model-config-provider-name"
                  v-model="modelConfig.name"
                  type="text"
                  :disabled="modelConfig.loading"
                />
              </div>
              <div class="setting-row">
                <label for="model-config-base-url">base_url</label>
                <input
                  id="model-config-base-url"
                  v-model="modelConfig.base_url"
                  type="text"
                  :disabled="modelConfig.loading"
                  placeholder="https://api.deepseek.com/"
                />
              </div>
              <div class="setting-row">
                <label for="model-config-token">experimental_bearer_token</label>
                <input
                  id="model-config-token"
                  v-model="modelConfig.experimental_bearer_token"
                  type="text"
                  :disabled="modelConfig.loading"
                  placeholder="你的 DeepSeek API Key"
                />
              </div>
            </div>

            <div class="model-config-actions">
              <button
                class="btn primary"
                :disabled="modelConfig.savingConfig || modelConfig.loading"
                @click="saveModelConfig"
              >
                {{ modelConfig.savingConfig ? "保存中…" : "保存" }}
              </button>
            </div>
          </div>

          <div class="model-config-card">
            <div class="model-config-card-head">
              <h3>models.json</h3>
              <div class="model-config-head-right">
                <button
                  v-if="modelConfig.models_json_path && modelConfig.models_json_exists"
                  type="button"
                  class="model-config-path-link"
                  title="在编辑器中打开文件"
                  @click="openModelsJsonFile"
                >
                  {{ modelConfig.models_json_path }}
                </button>
                <div v-else class="model-config-path">
                  {{ modelConfig.models_json_path || "正在读取路径…" }}
                  <span v-if="modelConfig.models_json_path && !modelConfig.models_json_exists" class="model-config-missing">
                    （文件不存在，保存时将新建）
                  </span>
                </div>
                <button
                  class="model-config-refresh-btn"
                  type="button"
                  title="重新从磁盘读取"
                  :disabled="modelConfig.loading"
                  @click="refreshModelsJson"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_REFRESH" />
                  </svg>
                </button>
              </div>
            </div>
            <textarea
              v-model="modelConfig.models_json"
              class="model-config-textarea"
              :disabled="modelConfig.loading"
              placeholder="在此编辑 models.json 文件内容（必须是合法 JSON）"
              spellcheck="false"
            ></textarea>
            <div class="model-config-actions">
              <button
                class="btn primary"
                :disabled="modelConfig.savingModelsJson || modelConfig.loading"
                @click="saveModelsJson"
              >
                {{ modelConfig.savingModelsJson ? "保存中…" : "保存" }}
              </button>
            </div>
          </div>

          <div class="model-config-card">
            <div class="model-config-card-head">
              <h3>AGENTS.md</h3>
              <div class="model-config-head-right">
                <button
                  v-if="agents.agents_path && agents.exists"
                  type="button"
                  class="model-config-path-link"
                  title="在编辑器中打开文件"
                  @click="openAgentsFile"
                >
                  {{ agents.agents_path }}
                </button>
                <div v-else class="model-config-path">
                  {{ agents.agents_path || "正在读取路径…" }}
                  <span v-if="agents.agents_path && !agents.exists" class="model-config-missing">
                    （文件不存在，保存时将新建）
                  </span>
                </div>
                <button
                  class="model-config-refresh-btn"
                  type="button"
                  title="重新从磁盘读取"
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
              placeholder="在此编辑 AGENTS.md 内容（Codex 全局自定义指令）"
              spellcheck="false"
            ></textarea>
            <div class="model-config-actions">
              <button
                class="btn primary"
                :disabled="agents.saving || agents.loading"
                @click="saveCustomInstructions"
              >
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
                重置记忆…
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
                选择文件…
              </button>
              <button
                v-if="codexPath"
                class="btn danger codex-clear-btn"
                @click="clearCodexPath()"
              >
                清除
              </button>
            </div>
            <p v-if="!codexPath && store.server.codexPath" class="setting-note">
              当前使用（自动检测）：{{ store.server.codexPath }}
            </p>
          </div>
          </div>
        </section>

        <section
          v-show="activeSection === 'plugins'"
          class="settings-section settings-section-plugins"
        >
          <h2 class="settings-section-title">插件管理</h2>
          <div class="plugin-manage">
          <div class="plugin-manage-toolbar">
            <button
              class="btn"
              :disabled="pluginState.loading"
              @click="refreshPlugins(true)"
            >
              {{ pluginState.loading ? "刷新中…" : "刷新目录" }}
            </button>
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
                  class="btn danger plugin-market-remove"
                  @click.stop="doRemoveMarketplace(mp)"
                >
                  移除市场
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
                      {{ pluginState.busy[p.id] ? "安装中…" : "安装" }}
                    </button>
                    <button
                      v-else
                      class="btn danger plugin-uninstall-btn"
                      :disabled="!!pluginState.busy[p.id]"
                      @click="doUninstall(p)"
                    >
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
