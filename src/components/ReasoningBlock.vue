<script setup lang="ts">
import { computed, ref } from "vue";
import { watch } from "vue";
import type { ThreadItem } from "../lib/types";
import { useElapsed } from "../composables/useElapsed";
import { useThrottledRef } from "../composables/useThrottledRef";
import { formatDuration, formatElapsed } from "../lib/format";
import { ICON_THINK } from "../lib/icons";

const props = defineProps<{ item: ThreadItem }>();
const open = ref(false);

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

// 折叠态单行预览：取首行非空内容
const previewLine = computed(() => {
  const t = shownText.value;
  if (!t) return "";
  return t.split("\n").find((l) => l.trim())?.trim() ?? "";
});

// 流式期间最多每 80ms 刷新一次内容，结束时立即刷净
const { ref: shownText, flush: flushText } = useThrottledRef(text, 80);
watch(
  () => props.item.streaming,
  (s) => {
    if (!s) flushText();
  },
);

</script>

<template>
  <div
    class="assistant-card assistant-card--reasoning"
    :class="{ expanded: open }"
  >
    <div class="assistant-card-header">
      <button
        type="button"
        class="assistant-card-toggle"
        :aria-expanded="open"
        :aria-label="'思考过程'"
        @click="open = !open"
      >
        <span class="assistant-card-arrow">{{ open ? "▾" : "▸" }}</span>
        <svg class="assistant-card-icon" viewBox="0 0 24 24" aria-hidden="true">
          <path :d="ICON_THINK" />
        </svg>
        <span>{{ open ? "收起思考过程" : "显示思考过程" }}</span>
        <span v-if="timeLabel" class="reasoning-time">{{ timeLabel }}</span>
      </button>
    </div>
    <div
      v-if="!open && previewLine"
      class="reasoning-preview"
      v-tooltip="'点击展开思考过程'"
      @click="open = true"
    >
      {{ previewLine }}
    </div>
    <Transition name="assistant-card-body">
      <div v-if="open" class="assistant-card-body reasoning-content">
        {{ shownText }}
      </div>
    </Transition>
  </div>
</template>
