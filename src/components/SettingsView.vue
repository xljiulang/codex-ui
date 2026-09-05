<script setup lang="ts">
import { ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  askConfirm,
  loadMemoryConfig,
  saveMemoryConfig,
  saveSettings,
  setToast,
  store,
  toastError,
} from "../composables/useCodex";
import {
  THEMES,
  previewTheme,
  type ThemeId,
} from "../composables/useTheme";
import {
  ICON_CHECK,
  ICON_DELETE,
  ICON_EXTENSION,
  ICON_FILE,
  ICON_FOLDER_OPEN,
  ICON_MCP,
  ICON_MODEL_CUBE,
  ICON_PALETTE,
  ICON_SKILL,
  ICON_TOOL,
  ICON_TUNE,
} from "../lib/icons";
import { PERMISSION_MODES } from "../lib/permissions";
import type {
  AppSettings,
  TerminalShell,
  FollowupMode,
  PermissionId,
} from "../lib/types";
import AppSelect, { type AppSelectOption } from "./AppSelect.vue";
import DynamicToolsSection from "./settings/DynamicToolsSection.vue";
import GlobalInstructionsSection from "./settings/GlobalInstructionsSection.vue";
import McpSection from "./settings/McpSection.vue";
import ModelConfigSection from "./settings/ModelConfigSection.vue";
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

/** 即时保存：任何设置项变更立即持久化（成功静默，失败 toast） */
async function persist(patch: Partial<AppSettings>) {
  try {
    await saveSettings(patch);
  } catch (e) {
    setToast(toastError(e));
  }
}

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

        <ModelConfigSection :active="activeSection === 'model-config'" />

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
