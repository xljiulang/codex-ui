<script setup lang="ts">
import { nextTick, ref, watch } from "vue";
import { tooltip } from "../composables/useTooltip";

const el = ref<HTMLElement | null>(null);

// 锚定在目标元素上方居中；上方放不下则放到下方，并收进视口
watch(
  () => [tooltip.visible, tooltip.text] as const,
  () => {
    if (!tooltip.visible || !tooltip.anchor) return;
    const anchor = tooltip.anchor;
    void nextTick(() => {
      const node = el.value;
      if (!node) return;
      const rect = node.getBoundingClientRect();
      const pad = 8;
      let x = anchor.left + anchor.width / 2 - rect.width / 2;
      let y = anchor.top - rect.height - 8;
      if (y < pad) {
        y = anchor.top + anchor.height + 8;
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
