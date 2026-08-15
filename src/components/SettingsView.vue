<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { call } from "../lib/ipc";
import {
  askConfirm,
  saveSettings,
  setToast,
  store,
  toastError,
} from "../composables/useCodex";
import {
  THEMES,
  applyTheme,
  previewTheme,
  type ThemeId,
} from "../composables/useTheme";
import { PERMISSION_MODES } from "../lib/permissions";

const closeBtn = ref<HTMLButtonElement | null>(null);
const codexPath = ref(store.settings.codex_path ?? "");
const sound = ref(store.settings.sound_enabled);
const enterToSend = ref(store.settings.enter_to_send);
const followupMode = ref(store.settings.followup_mode);
const theme = ref<ThemeId>(store.settings.theme as ThemeId);
const defaultPermission = ref(store.settings.default_permission);
const memoryMode = ref(store.settings.memory_mode);

function close(restore = true) {
  if (restore && theme.value !== store.settings.theme) {
    applyTheme(store.settings.theme);
  }
  store.showSettings = false;
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") close();
}

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
  void nextTick(() => closeBtn.value?.focus());
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  if (theme.value !== store.settings.theme) {
    applyTheme(store.settings.theme);
  }
});

async function pickCodexFile() {
  try {
    const current = codexPath.value.trim();
    const initialDir = current
      ? current.replace(/[\\/][^\\/]*$/, "")
      : undefined;
    const dir = await call<string | null>("pick_codex_file", {
      initialDir,
    });
    if (dir) codexPath.value = dir;
  } catch (e) {
    setToast(toastError(e));
  }
}

async function apply() {
  await saveSettings({
    codex_path: codexPath.value.trim() || null,
    sound_enabled: sound.value,
    enter_to_send: enterToSend.value,
    followup_mode: followupMode.value,
    theme: theme.value,
    default_permission: defaultPermission.value,
    memory_mode: memoryMode.value,
  });
  // 有当前会话时立即同步记忆模式（与模型同步一致）：失败 toast 但不阻塞保存
  let syncError: string | null = null;
  if (store.currentThreadId) {
    try {
      await call("codex_rpc", {
        method: "thread/memoryMode/set",
        params: { threadId: store.currentThreadId, mode: memoryMode.value },
      });
    } catch (e) {
      syncError = toastError(e);
    }
  }
  setToast(syncError ?? "设置已保存");
  close(false);
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
    await call("codex_rpc", { method: "memory/reset", params: null });
    setToast("记忆已重置");
  } catch (e) {
    setToast(toastError(e));
  }
}

function selectTheme(id: ThemeId) {
  // 更新本地暂存并即时预览（保存才持久化，取消/关闭时还原）
  theme.value = id;
  previewTheme(id);
}
</script>

<template>
  <div class="modal-mask">
    <div
      class="modal settings-modal"
      role="dialog"
      aria-modal="true"
      aria-label="设置"
    >
      <div class="modal-head">
        <span class="modal-title">设置</span>
        <button
          ref="closeBtn"
          class="modal-close"
          aria-label="关闭设置"
          @click="close()"
        >
          ×
        </button>
      </div>
      <div class="modal-body">
        <div class="settings">
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

          <div class="setting-row checkbox-row">
            <input id="sound" v-model="sound" type="checkbox" />
            <label for="sound" style="margin: 0">提权/交互时播放提示音</label>
          </div>

          <div class="setting-row checkbox-row">
            <input id="enter" v-model="enterToSend" type="checkbox" />
            <label for="enter" style="margin: 0">
              Enter 快捷发送（开启时 Ctrl+Enter 换行；关闭后 Enter 换行，Ctrl+Enter 发送）
            </label>
          </div>

          <div class="setting-row">
            <label>跟进处理方式</label>
            <select v-model="followupMode">
              <option value="adjust">调整方向</option>
              <option value="queue">加入队列</option>
            </select>
          </div>

          <div class="setting-row">
            <label>默认权限</label>
            <select v-model="defaultPermission" class="default-permission-select">
              <option v-for="m in PERMISSION_MODES" :key="m.id" :value="m.id">
                {{ m.label }}
              </option>
            </select>
          </div>

          <div class="setting-row">
            <label>记忆模式</label>
            <select v-model="memoryMode" class="memory-mode-select">
              <option value="disabled">关闭</option>
              <option value="enabled">启用</option>
            </select>
          </div>

          <div class="setting-row">
            <label>重置记忆</label>
            <div class="setting-path-row">
              <div class="setting-value">清空全部已保存的记忆（无法撤销）</div>
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
                @click="codexPath = ''"
              >
                清除
              </button>
            </div>
            <p v-if="!codexPath && store.server.codexPath" class="setting-note">
              当前使用（自动检测）：{{ store.server.codexPath }}
            </p>
          </div>
        </div>
      </div>
      <div class="modal-foot">
        <button class="btn" @click="close()">取消</button>
        <button class="btn primary" @click="apply()">保存</button>
      </div>
    </div>
  </div>
</template>
