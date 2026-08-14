<script setup lang="ts">
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { openNewSession, setToast, store, toastError } from "../composables/useCodex";
import { ICON_PLUS } from "../lib/icons";

// 选择文件夹对话框打开中：禁止重复触发，避免同时弹多个系统对话框
const picking = ref(false);

async function onNewChat() {
  if (picking.value) return;
  picking.value = true;
  try {
    // 先选目录；取消选择文件夹则流程直接结束（不新建、不聚焦、不切 Tab）
    const dir = await invoke<string | null>("pick_directory");
    if (!dir) return;
    await openNewSession(dir);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    picking.value = false;
  }
}

function onSettings() {
  store.showSettings = !store.showSettings;
}
</script>

<template>
  <header class="app-header">
    <div class="brand">
      <span class="brand-logo">
        <svg viewBox="0 0 24 24">
          <path d="M12 2l8.66 5v10L12 22l-8.66-5V7z" />
          <path class="logo-c" d="M14.9 9.1a4.5 4.5 0 1 0 0 5.8" />
        </svg>
      </span>
      <span class="brand-name">CODEX</span>
    </div>
    <div class="header-actions">
      <button
        class="icon-btn new-chat-btn"
        aria-label="新建会话"
        :disabled="picking"
        v-tooltip="picking ? '正在选择文件夹…' : '新建会话（选择工作目录）'"
        @click="onNewChat()"
      >
        <svg viewBox="0 0 24 24">
          <path :d="ICON_PLUS" />
        </svg>
      </button>
      <button
        class="icon-btn"
        aria-label="设置"
        v-tooltip="'设置'"
        @click="onSettings()"
      >
        <svg viewBox="0 0 24 24">
          <path
            d="M19.14 12.94a7.1 7.1 0 0 0 .06-.94 7.1 7.1 0 0 0-.06-.94l2.03-1.58a.5.5 0 0 0 .12-.64l-1.92-3.32a.5.5 0 0 0-.61-.22l-2.39.96a7.1 7.1 0 0 0-1.62-.94L14.4 2.8a.5.5 0 0 0-.49-.4h-3.82a.5.5 0 0 0-.49.4l-.36 2.54a7.1 7.1 0 0 0-1.62.94l-2.39-.96a.5.5 0 0 0-.61.22l-1.92 3.32a.5.5 0 0 0 .12.64l2.03 1.58a7.1 7.1 0 0 0 0 1.88l-2.03 1.58a.5.5 0 0 0-.12.64l1.92 3.32a.5.5 0 0 0 .61.22l2.39-.96a7.1 7.1 0 0 0 1.62.94l.36 2.54a.5.5 0 0 0 .49.4h3.82a.5.5 0 0 0 .49-.4l.36-2.54a7.1 7.1 0 0 0 1.62-.94l2.39.96a.5.5 0 0 0 .61-.22l1.92-3.32a.5.5 0 0 0-.12-.64zM12 15.5A3.5 3.5 0 1 1 15.5 12 3.5 3.5 0 0 1 12 15.5z"
          />
        </svg>
      </button>
    </div>
  </header>
</template>
