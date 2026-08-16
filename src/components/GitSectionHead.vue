<script setup lang="ts">
import { ICON_ARROW_DOWN, ICON_ARROW_RIGHT } from "../lib/icons";

defineProps<{
  label: string;
  collapsed: boolean;
}>();

const emit = defineEmits<{ toggle: [] }>();
</script>

<template>
  <div
    class="git-section-head"
    role="button"
    tabindex="0"
    :aria-expanded="!collapsed"
    @click="emit('toggle')"
    @keydown.enter="emit('toggle')"
  >
    <svg class="git-section-arrow" viewBox="0 0 24 24" aria-hidden="true">
      <path :d="collapsed ? ICON_ARROW_RIGHT : ICON_ARROW_DOWN" />
    </svg>
    <span>{{ label }}</span>
    <span class="git-section-actions">
      <slot />
    </span>
  </div>
</template>
