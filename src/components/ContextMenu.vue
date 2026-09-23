<script setup lang="ts">
import type { ActionMenuItem } from "../composables/useActionMenu";

defineProps<{
  items: ActionMenuItem[];
  x: number;
  y: number;
  /** 菜单项整体禁用（如 Git 操作进行中） */
  disabled?: boolean;
}>();

const emit = defineEmits<{ close: [] }>();
</script>

<template>
  <div class="ctx-menu" :style="{ left: x + 'px', top: y + 'px' }" @click.stop>
    <button
      v-for="it in items"
      :key="it.label"
      class="ctx-menu-item popup-menu-item"
      :class="{ danger: it.danger }"
      :disabled="disabled"
      @click="
        it.action();
        emit('close');
      "
    >
      <img
        v-if="it.img"
        class="ctx-menu-item-img"
        :src="it.img"
        alt=""
        draggable="false"
      />
      <svg
        v-else-if="it.paths || it.icon"
        viewBox="0 0 24 24"
        aria-hidden="true"
        :class="{ 'ctx-session-logo': it.paths }"
      >
        <template v-if="it.paths">
          <path
            v-for="p in it.paths"
            :key="p.d"
            :d="p.d"
            :class="{ 'logo-c': p.accent }"
          />
        </template>
        <path
          v-else
          :d="it.icon"
          :fill-rule="it.nonzero ? 'nonzero' : 'evenodd'"
          :fill="it.stroke ? 'none' : 'currentColor'"
          :stroke="it.stroke ? 'currentColor' : 'none'"
          :stroke-width="it.stroke ? 2 : undefined"
          vector-effect="non-scaling-stroke"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
      <span>{{ it.label }}</span>
    </button>
  </div>
</template>

<style scoped>
.ctx-menu {
  position: fixed;
  z-index: 1100;
  min-width: 140px;
  background: var(--float-bg);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius);
  box-shadow: var(--shadow-lg);
  padding: var(--space-1);
  display: flex;
  flex-direction: column;
  animation: menu-in var(--ease);
}

.ctx-menu-item {
  gap: var(--space-3);
  padding: var(--space-3) 11px;
}

.ctx-menu-item svg {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  fill: currentColor;
}

.ctx-menu-item-img {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  object-fit: contain;
}

.ctx-menu-item.danger:hover {
  color: var(--red);
}
</style>
