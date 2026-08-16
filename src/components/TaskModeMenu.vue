<script setup lang="ts">
import { activeSessionTab } from "../composables/useCodex";
import { TASK_MODES, type TaskMode } from "../lib/tasks";

const emit = defineEmits<{ close: [] }>();

function choose(id: TaskMode["id"]) {
  const tab = activeSessionTab();
  if (tab?.turnActive) return; // 回合进行中不可切换（按钮本身已禁用，这里兜底）
  if (tab) tab.taskMode = id;
  emit("close");
}
</script>

<template>
  <div class="popup-menu" @click.stop>
    <div class="menu-group-title" style="padding: 6px 8px">任务模式</div>
    <button
      v-for="m in TASK_MODES"
      :key="m.id"
      class="mode-menu-item"
      :class="{ selected: activeSessionTab()?.taskMode === m.id }"
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
      <span v-if="activeSessionTab()?.taskMode === m.id" class="mode-check">✓</span>
    </button>
  </div>
</template>
