<script setup lang="ts">
import { PERMISSION_MODES } from "../lib/permissions";
import { store } from "../composables/useCodex";

const emit = defineEmits<{ close: [] }>();

function choose(id: string) {
  if (store.turnActive) return; // 回合进行中不可切换（按钮本身已禁用，这里兜底）
  store.permissionMode = id; // 进程级生效，不写配置文件
  emit("close");
}
</script>

<template>
  <div class="popup-menu" @click.stop>
    <div class="menu-group-title" style="padding: 6px 8px">
      应如何批准 Codex 操作？
    </div>
    <button
      v-for="m in PERMISSION_MODES"
      :key="m.id"
      class="mode-menu-item"
      :class="{ selected: store.permissionMode === m.id }"
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
      <span v-if="store.permissionMode === m.id" class="mode-check">✓</span>
    </button>
  </div>
</template>
