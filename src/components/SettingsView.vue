<script setup lang="ts">
import { ref } from "vue";
import { saveSettings, store } from "../composables/useCodex";

const codexPath = ref(store.settings.codex_path ?? "");
const sound = ref(store.settings.sound_enabled);
const enterToSend = ref(store.settings.enter_to_send);

async function apply() {
  await saveSettings({
    codex_path: codexPath.value.trim() || null,
    sound_enabled: sound.value,
    enter_to_send: enterToSend.value,
  });
  store.toast = "设置已保存";
}
</script>

<template>
  <div class="settings">
    <div class="setting-row" style="display: flex; align-items: center; justify-content: space-between">
      <h2 style="margin: 0">设置</h2>
      <button class="btn" @click="store.showSettings = false">返回聊天</button>
    </div>

    <div class="setting-row">
      <label>codex 可执行文件路径（留空使用 PATH）</label>
      <input v-model="codexPath" type="text" placeholder="例如 C:\Users\you\.local\bin\codex.exe" />
    </div>

    <div class="setting-row checkbox-row">
      <input id="sound" v-model="sound" type="checkbox" />
      <label for="sound" style="margin: 0">提权/交互时播放提示音</label>
    </div>

    <div class="setting-row checkbox-row">
      <input id="enter" v-model="enterToSend" type="checkbox" />
      <label for="enter" style="margin: 0">
        Enter 快捷发送（关闭后 Enter 换行，Ctrl+Enter 发送）
      </label>
    </div>

    <div class="setting-row">
      <button class="btn primary" @click="apply()">保存设置</button>
    </div>
  </div>
</template>
