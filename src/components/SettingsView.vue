<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { saveSettings, store, toastError } from "../composables/useCodex";
import { applyTheme, THEMES, type ThemeId } from "../composables/useTheme";

const closeBtn = ref<HTMLButtonElement | null>(null);
const codexPath = ref(store.settings.codex_path ?? "");
const sound = ref(store.settings.sound_enabled);
const enterToSend = ref(store.settings.enter_to_send);
const followupMode = ref(store.settings.followup_mode);

function close() {
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
});

async function pickCodexFile() {
  try {
    const current = codexPath.value.trim();
    const initialDir = current
      ? current.replace(/[\\/][^\\/]*$/, "")
      : undefined;
    const dir = await invoke<string | null>("pick_codex_file", {
      initialDir,
    });
    if (dir) codexPath.value = dir;
  } catch (e) {
    store.toast = toastError(e);
  }
}

async function apply() {
  await saveSettings({
    codex_path: codexPath.value.trim() || null,
    sound_enabled: sound.value,
    enter_to_send: enterToSend.value,
    followup_mode: followupMode.value,
  });
  store.toast = "设置已保存";
  close();
}

async function selectTheme(id: ThemeId) {
  store.settings.theme = id;
  applyTheme(id);
  try {
    await saveSettings({ theme: id });
  } catch (e) {
    store.toast = toastError(e);
  }
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
                :class="{ selected: store.settings.theme === t.id }"
                :data-theme-id="t.id"
                :aria-pressed="store.settings.theme === t.id"
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
            <label>codex 可执行文件（留空使用 PATH）</label>
            <div class="setting-path-row">
              <div class="setting-value">
                {{ codexPath || "未设置（使用 PATH 查找）" }}
              </div>
              <button class="btn" @click="pickCodexFile()">选择文件…</button>
              <button v-if="codexPath" class="btn danger" @click="codexPath = ''">
                清除
              </button>
            </div>
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
