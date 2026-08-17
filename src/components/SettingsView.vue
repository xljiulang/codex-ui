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
import { ICON_CHEVRON_DOWN } from "../lib/icons";
import { PERMISSION_MODES } from "../lib/permissions";
import type { AppSettings, TerminalShell } from "../lib/types";

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
  { id: "general", label: "通用设置" },
  { id: "plugins", label: "插件管理" },
] as const;
type SettingsSectionId = (typeof settingsSections)[number]["id"];
/** 当前选中分类：默认取第一个分类（不依赖具体标签），重开设置页（v-if 重新挂载）重置 */
const activeSection = ref<SettingsSectionId>(settingsSections[0].id);

onMounted(() => {
  void refreshPlugins();
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
          v-show="activeSection === 'general'"
          class="settings-section settings-section-general"
        >
          <h2 class="settings-section-title">通用</h2>
          <div class="settings">

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
