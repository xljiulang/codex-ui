<script setup lang="ts">
import {
  openSettingsTab,
  SETTINGS_TAB_ID,
} from "../composables/useEditorTabs";
import { activateTab, tabs } from "../composables/useTabs";
import { ICON_SETTINGS } from "../lib/icons";

/** 设置按钮：无设置标签则创建并激活；已存在（无论是否激活）仅聚焦激活 */
function onSettings() {
  const existing = tabs.find((t) => t.id === SETTINGS_TAB_ID);
  if (!existing) {
    openSettingsTab();
    return;
  }
  activateTab(SETTINGS_TAB_ID);
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
