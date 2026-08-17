<script setup lang="ts">
import {
  pickAndOpenNewSession,
  pickingNewSessionDir,
} from "../composables/useCodex";
import {
  closeAnyTab,
  openSettingsTab,
  SETTINGS_TAB_ID,
} from "../composables/useEditorTabs";
import { activeTabId, activateTab, tabs } from "../composables/useTabs";
import { ICON_PLUS, ICON_SETTINGS } from "../lib/icons";

/** 设置按钮：无设置标签则创建并激活；已存在未激活则激活；已激活则关闭（保持原 toggle 习惯） */
function onSettings() {
  const existing = tabs.find((t) => t.id === SETTINGS_TAB_ID);
  if (!existing) {
    openSettingsTab();
    return;
  }
  if (activeTabId.value === SETTINGS_TAB_ID) {
    void closeAnyTab(existing);
  } else {
    activateTab(SETTINGS_TAB_ID);
  }
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
        :disabled="pickingNewSessionDir"
        v-tooltip="
          pickingNewSessionDir
            ? '正在选择文件夹…'
            : '新建会话（选择工作目录）'
        "
        @click="pickAndOpenNewSession()"
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
          <path :d="ICON_SETTINGS" />
        </svg>
      </button>
    </div>
  </header>
</template>
