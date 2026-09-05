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
} from "../../composables/useCodex";
import {
  ICON_DELETE,
  ICON_FOLDER_OPEN,
} from "../../lib/icons";
import { PERMISSION_MODES } from "../../lib/permissions";
import type {
  AppSettings,
  TerminalShell,
  FollowupMode,
  PermissionId,
} from "../../lib/types";
import AppSelect, { type AppSelectOption } from "../AppSelect.vue";

const props = defineProps<{ active: boolean }>();

const codexPath = ref(store.settings.codex_path ?? "");
const terminalShell = ref<TerminalShell>(store.settings.terminal_shell);
const defaultPermission = ref(store.settings.default_permission);
const followupMode = ref(store.settings.followup_mode);

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

// ---------- 本地记忆 ----------

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

// 进入「基础设置」分区时从 codex 配置回填记忆开关
watch(
  () => props.active,
  (active) => {
    if (active) void loadMemorySection();
  },
);
</script>

<template>
  <section v-show="active" class="settings-section settings-section-basic">
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
</template>
