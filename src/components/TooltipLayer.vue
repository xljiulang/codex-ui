<script setup lang="ts">
import { nextTick, ref, watch } from "vue";
import { tooltip } from "../composables/useTooltip";

const el = ref<HTMLElement | null>(null);

// 锚定在目标元素上方居中；上方放不下则放到下方，并收进视口
watch(
  () => [tooltip.visible, tooltip.text, tooltip.anchor, tooltip.placement] as const,
  () => {
    if (!tooltip.visible || !tooltip.anchor) return;
    const anchor = tooltip.anchor;
    void nextTick(() => {
      const node = el.value;
      if (!node) return;
      const rect = node.getBoundingClientRect();
      const pad = 8;
      let x: number;
      let y: number;
      if (tooltip.placement === "left") {
        x = anchor.left - rect.width - pad;
        y = anchor.top + anchor.height / 2 - rect.height / 2;
      } else {
        x = anchor.left + anchor.width / 2 - rect.width / 2;
        y = anchor.top - rect.height - pad;
        if (y < pad) {
          y = anchor.top + anchor.height + pad;
        }
      }
      x = Math.min(Math.max(pad, x), window.innerWidth - rect.width - pad);
      if (y + rect.height > window.innerHeight - pad) {
        y = window.innerHeight - rect.height - pad;
      }
      if (y < pad) y = pad;
      tooltip.x = x;
      tooltip.y = y;
    });
  },
);
</script>

<template>
  <div
    v-if="tooltip.visible"
    ref="el"
    class="app-tooltip"
    role="tooltip"
    :style="{ left: tooltip.x + 'px', top: tooltip.y + 'px' }"
  >
    {{ tooltip.text }}
  </div>
</template>

<style scoped>
.app-tooltip {
  position: fixed;
  z-index: 500;
  max-width: 340px;
  padding: var(--space-3) 11px;
  border-radius: var(--radius);
  font-size: var(--font-sm);
  line-height: 1.5;
  color: var(--text-bright);
  background: var(--float-bg);
  border: 1px solid var(--glass-border);
  box-shadow: var(--shadow-md);
  pointer-events: none;
  word-break: break-word;
  white-space: pre-wrap;
  animation: tooltip-in var(--ease);
}
</style>
