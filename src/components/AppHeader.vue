<script setup lang="ts">
import {
  openSettingsTab,
  SETTINGS_TAB_ID,
} from "../composables/useEditorTabs";
import { activateTab, tabs } from "../composables/useTabs";
import { useWindowControls } from "../composables/useWindowControls";
import {
  ICON_SETTINGS,
  ICON_CLOSE,
  ICON_LAYOUT_SIDE,
  ICON_LAYOUT_SIDE_HIDDEN,
  ICON_WINDOW_MIN,
  ICON_WINDOW_MAX,
  ICON_WINDOW_RESTORE,
} from "../lib/icons";
import { store } from "../composables/useCodex";

// 自绘标题栏窗口控制：最小化/最大化(还原)/关闭 + 拖动窗口 + 双击最大化
const {
  isMaximized,
  minimize,
  toggleMaximize,
  close,
  onTitlebarMouseDown,
} = useWindowControls();

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
  <header
    class="app-header"
    @mousedown="onTitlebarMouseDown"
  >
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
      <button
        class="icon-btn"
        aria-label="切换布局"
        v-tooltip="'切换布局'"
        @click="store.rightPanelHidden = !store.rightPanelHidden"
      >
        <svg viewBox="0 0 24 24">
          <path
            :d="store.rightPanelHidden ? ICON_LAYOUT_SIDE_HIDDEN : ICON_LAYOUT_SIDE"
            fill-rule="evenodd"
          />
        </svg>
      </button>
      <button
        class="icon-btn win-btn"
        aria-label="最小化"
        @click="minimize()"
      >
        <svg viewBox="0 0 24 24">
          <path :d="ICON_WINDOW_MIN" />
        </svg>
      </button>
      <button
        class="icon-btn win-btn"
        :aria-label="isMaximized ? '还原' : '最大化'"
        @click="toggleMaximize()"
      >
        <svg viewBox="0 0 24 24">
          <path :d="isMaximized ? ICON_WINDOW_RESTORE : ICON_WINDOW_MAX" />
        </svg>
      </button>
      <button
        class="icon-btn win-btn win-btn-close"
        aria-label="关闭"
        @click="close()"
      >
        <svg viewBox="0 0 24 24">
          <path :d="ICON_CLOSE" />
        </svg>
      </button>
    </div>
  </header>
</template>
