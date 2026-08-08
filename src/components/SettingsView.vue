<script setup lang="ts">
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  refreshServer,
  saveSettings,
  store,
} from "../composables/useCodex";

const codexPath = ref(store.settings.codex_path ?? "");
const sound = ref(store.settings.sound_enabled);
const logs = ref<string[]>([]);

async function apply() {
  await saveSettings({
    codex_path: codexPath.value.trim() || null,
    sound_enabled: sound.value,
  });
  store.toast = "设置已保存";
}

async function reconnect() {
  try {
    await invoke("server_connect");
    await refreshServer();
    store.toast = "已触发重连";
  } catch (e) {
    store.toast = String(e);
  }
}

async function showLogs() {
  try {
    logs.value = await invoke<string[]>("server_logs");
  } catch {
    logs.value = [];
  }
}

void showLogs();
</script>

<template>
  <div class="settings">
    <div class="setting-row" style="display: flex; align-items: center; justify-content: space-between">
      <h2 style="margin: 0">设置</h2>
      <button class="btn" @click="store.showSettings = false">返回聊天</button>
    </div>

    <div class="setting-row">
      <label>工作目录（codex 工作目录）</label>
      <div class="setting-value">{{ store.server.workspace }}</div>
    </div>

    <div class="setting-row">
      <label>codex 可执行文件路径（留空使用 PATH）</label>
      <input v-model="codexPath" type="text" placeholder="例如 C:\Users\you\.local\bin\codex.exe" />
    </div>

    <div class="setting-row checkbox-row">
      <input id="sound" v-model="sound" type="checkbox" />
      <label for="sound" style="margin: 0">提权/交互时播放提示音</label>
    </div>

    <div class="setting-row">
      <label>服务状态</label>
      <div class="setting-value" :style="store.server.connected ? 'color: var(--green)' : 'color: var(--red)'">
        {{ store.server.connected ? "已连接" : "未连接" }}
        <span v-if="store.server.codexPath">（{{ store.server.codexPath }}）</span>
      </div>
    </div>

    <div class="setting-row">
      <button class="btn" @click="reconnect()">重新连接</button>
      <button class="btn" style="margin-left: 8px" @click="showLogs()">刷新日志</button>
    </div>

    <div class="setting-row">
      <label>服务日志</label>
      <pre class="server-logs">{{ logs.join("\n") || "暂无日志" }}</pre>
    </div>

    <div class="setting-row">
      <button class="btn primary" @click="apply()">保存设置</button>
    </div>
  </div>
</template>
