<script setup lang="ts">
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { saveSettings, store, toastError } from "../composables/useCodex";

const codexPath = ref(store.settings.codex_path ?? "");
const sound = ref(store.settings.sound_enabled);
const enterToSend = ref(store.settings.enter_to_send);
const followupMode = ref(store.settings.followup_mode);

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
}
</script>

<template>
  <div class="settings">
    <div class="setting-row">
      <h2 style="margin: 0">设置</h2>
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

    <div class="setting-row">
      <button class="btn primary" @click="apply()">保存设置</button>
    </div>
  </div>
</template>
