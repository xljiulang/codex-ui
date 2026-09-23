<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
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

// 折叠态单行预览：取最后一个非空行（流式期间实时跟随最新写入的内容）
const previewLine = computed(() => {
  const lines = shownText.value.split("\n");
  for (let i = lines.length - 1; i >= 0; i--) {
    const line = lines[i].trim();
    if (line) return line;
  }
  return "";
});

// 流式期间最多每 80ms 刷新一次内容，结束时立即刷净
const { ref: shownText, flush: flushText } = useThrottledRef(text, 80);
watch(
  () => props.item.streaming,
  (s) => {
    if (!s) flushText();
  },
);

// 预览行超出可视宽度时把窗口顶到最右：视觉上内容不断向左滑动、始终露出最新字符
const previewEl = ref<HTMLElement | null>(null);
const previewClipped = ref(false);
let previewObserver: ResizeObserver | undefined;

function syncPreviewScroll() {
  const el = previewEl.value;
  const clipped = !!el && el.scrollWidth - el.clientWidth > 1;
  if (el && clipped) el.scrollLeft = el.scrollWidth;
  previewClipped.value = clipped;
}

// 内容更新或重新收起后重新对齐到行尾
watch([shownText, open], () => {
  void nextTick(syncPreviewScroll);
});

// 预览行存在期间跟随容器宽度变化（窗口或面板尺寸调整）
watch(previewEl, (el) => {
  previewObserver?.disconnect();
  previewObserver = undefined;
  if (el && typeof ResizeObserver !== "undefined") {
    previewObserver = new ResizeObserver(() => syncPreviewScroll());
    previewObserver.observe(el);
  }
  syncPreviewScroll();
});

onBeforeUnmount(() => {
  previewObserver?.disconnect();
  previewObserver = undefined;
});
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
        <svg
          class="assistant-card-arrow"
          viewBox="0 0 24 24"
          aria-hidden="true"
        >
          <path :d="open ? ICON_ARROW_DOWN : ICON_ARROW_RIGHT" />
        </svg>
        <svg class="assistant-card-icon" viewBox="0 0 24 24" aria-hidden="true">
          <path :d="ICON_THINK" />
        </svg>
        <span class="assistant-card-title">思考过程</span>
        <span v-if="timeLabel" class="reasoning-time">{{ timeLabel }}</span>
      </button>
    </div>
    <div
      v-if="!open && previewLine"
      ref="previewEl"
      class="reasoning-preview"
      :class="{ 'is-clipped': previewClipped }"
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
  cursor: pointer;
  user-select: none;
}

/* 行尾跟随：内容超出一行时左端被裁掉，用渐隐提示（短行不会变淡） */
.reasoning-preview.is-clipped {
  -webkit-mask-image: linear-gradient(to right, transparent 0, #000 22px);
  mask-image: linear-gradient(to right, transparent 0, #000 22px);
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
