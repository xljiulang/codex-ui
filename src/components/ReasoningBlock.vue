<script setup lang="ts">
import { computed, ref } from "vue";
import { watch } from "vue";
import type { ThreadItem } from "../lib/types";
import { useElapsed } from "../composables/useElapsed";
import { formatDuration, formatElapsed } from "../lib/format";

const props = defineProps<{ item: ThreadItem }>();
const open = ref(false);

// 思考过程中默认展开，完成后自动折叠
watch(
  () => props.item.streaming,
  (streaming) => {
    open.value = streaming === true;
  },
  { immediate: true },
);

const streaming = computed(() => props.item.streaming === true);
const startedAt = computed(() =>
  typeof props.item.startedAtMs === "number"
    ? (props.item.startedAtMs as number)
    : Date.now(),
);
const { elapsed } = useElapsed(startedAt.value);
const durationMs = computed(() =>
  typeof props.item.durationMs === "number"
    ? (props.item.durationMs as number)
    : null,
);
const timeLabel = computed(() => {
  if (streaming.value) return formatElapsed(elapsed.value);
  if (durationMs.value != null) return formatDuration(durationMs.value);
  return "";
});

const text = computed(() => {
  const content = (props.item.content as string[] | undefined) ?? [];
  const summary = (props.item.summary as string[] | undefined) ?? [];
  const parts = content.length ? content : summary;
  return parts.join("\n");
});
</script>

<template>
  <div class="reasoning-block">
    <button class="reasoning-toggle" @click="open = !open">
      <svg
        viewBox="0 0 16 16"
        width="10"
        height="10"
        style="fill: currentColor; transform: rotate(0deg)"
        :style="open ? 'transform: rotate(90deg)' : ''"
      >
        <path d="M6 4l4 4-4 4z" />
      </svg>
      <span>{{ open ? "收起思考过程" : "显示思考过程" }}</span>
      <span v-if="timeLabel" class="reasoning-time">{{ timeLabel }}</span>
    </button>
    <div v-if="open" class="reasoning-content">{{ text }}</div>
  </div>
</template>
