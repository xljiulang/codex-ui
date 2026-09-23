<script setup lang="ts">
import { activeSessionTab } from "../composables/useCodex";
import {
  COLLABORATION_MODES,
  type CollaborationMode,
} from "../lib/collaborationModes";

const emit = defineEmits<{ close: [] }>();

function choose(id: CollaborationMode["id"]) {
  const tab = activeSessionTab();
  if (tab?.turnActive) return; // 回合进行中不可切换（按钮本身已禁用，这里兜底）
  if (tab) tab.collaborationMode = id;
  emit("close");
}
</script>

<template>
  <div class="popup-menu composer-menu" @click.stop>
    <div class="menu-group-title" style="padding: 6px 8px">协作模式</div>
    <button
      v-for="m in COLLABORATION_MODES"
      :key="m.id"
      class="mode-menu-item"
      :class="{ selected: activeSessionTab()?.collaborationMode === m.id }"
      @click="choose(m.id)"
    >
      <span class="mode-icon">
        <svg viewBox="0 0 24 24">
          <path :d="m.icon" />
        </svg>
      </span>
      <span>
        <div class="mode-label">{{ m.label }}</div>
        <div class="mode-desc">{{ m.desc }}</div>
      </span>
      <span
        v-if="activeSessionTab()?.collaborationMode === m.id"
        class="mode-check"
        >✓</span
      >
    </button>
  </div>
</template>
