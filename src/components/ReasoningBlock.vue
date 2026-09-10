<script setup lang="ts">
import { computed, ref } from "vue";
import { watch } from "vue";
import type { ThreadItem } from "../lib/types";
import { useElapsed } from "../composables/useElapsed";
import { useThrottledRef } from "../composables/useThrottledRef";
import { formatDuration, formatElapsed } from "../lib/format";
import { ICON_ARROW_DOWN, ICON_ARROW_RIGHT, ICON_THINK } from "../lib/icons";

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
        <svg class="assistant-card-arrow" viewBox="0 0 24 24" aria-hidden="true">
          <path :d="open ? ICON_ARROW_DOWN : ICON_ARROW_RIGHT" />
        </svg>
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

<style scoped>
/* 推理折叠块：容器走共享 .assistant-card，保留紫色语义边框与底色 */
.assistant-card--reasoning {
  border-left: 3px solid var(--purple);
  background: var(--reasoning-bg);
}

.reasoning-time {
  margin-left: auto;
  color: var(--text-faint);
  font-size: var(--font-xs);
  font-variant-numeric: tabular-nums;
}

.reasoning-preview {
  padding: 0 var(--space-5) var(--space-3);
  font-size: var(--font-sm);
  color: var(--text-dim);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
  user-select: none;
}

.reasoning-preview:hover {
  color: var(--text-bright);
}

.reasoning-content {
  color: var(--text-dim);
  font-size: var(--font-md);
  white-space: pre-wrap;
  user-select: text;
  line-height: 1.55;
}
</style>
