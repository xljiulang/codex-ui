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
      class="ctx-menu-item"
      :class="{ danger: it.danger }"
      :disabled="disabled"
      @click="it.action(); emit('close')"
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
