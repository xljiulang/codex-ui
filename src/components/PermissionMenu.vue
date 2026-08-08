<script setup lang="ts">
import { PERMISSION_MODES } from "../lib/permissions";
import { saveSettings, store } from "../composables/useCodex";

const emit = defineEmits<{ close: [] }>();

function choose(id: string) {
  if (store.turnActive) return; // 回合进行中不可切换（按钮本身已禁用，这里兜底）
  void saveSettings({ permission_mode: id });
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
      :class="{ selected: store.settings.permission_mode === m.id }"
      @click="choose(m.id)"
    >
      <span class="mode-icon">
        <svg v-if="m.icon === 'hand'" viewBox="0 0 24 24">
          <path
            d="M6.6 11h.2V6.5a1.5 1.5 0 0 1 3 0V10h.2V4.5a1.5 1.5 0 0 1 3 0V10h.2V6a1.5 1.5 0 0 1 3 0v7.6l-1.5 3.4A4 4 0 0 1 12 20H9.5a4 4 0 0 1-3.7-2.4L4 13.6a1.8 1.8 0 0 1 1.6-2.5z"
          />
        </svg>
        <svg v-else-if="m.icon === 'shield'" viewBox="0 0 24 24">
          <path
            d="M12 2 4 5v6c0 5.55 3.84 10.74 8 12 4.16-1.26 8-6.45 8-12V5l-8-3zm0 2.2 6 2.3V11c0 4.4-2.8 8.5-6 9.9C8.8 19.5 6 15.4 6 11V6.5l6-2.3zm.8 4.5-4.2 4.2 1.4 1.4 2.8-2.8 3.6 3.6 1.4-1.4z"
          />
        </svg>
        <svg v-else viewBox="0 0 24 24">
          <path
            d="M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm-1 15v-2h2v2zm0-4V7h2v6z"
          />
        </svg>
      </span>
      <span>
        <div class="mode-label">{{ m.label }}</div>
        <div class="mode-desc">{{ m.desc }}</div>
      </span>
      <span v-if="store.settings.permission_mode === m.id" class="mode-check">✓</span>
    </button>
  </div>
</template>
