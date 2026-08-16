<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref } from "vue";
import { openLink, sessionWorkspace } from "../lib/links";
import {
  NEW_CHAT_PLUGIN_KEY,
  activeSessionTab,
  store,
} from "../composables/useCodex";

const props = withDefaults(
  defineProps<{
    path: string;
    label: string;
    kind: "file" | "plugin" | "skill";
    delay?: number;
  }>(),
  { delay: 120 },
);

/** 本地路径（非 plugin:// 等 URI）可点击，点击用资源管理器定位 */
const clickable = computed(
  () => !!props.path && !props.path.startsWith("plugin://"),
);

/** 悬浮内容：文件显示路径；插件/技能显示说明（缓存缺失回退路径） */
const tipText = computed(() => {
  if (props.kind === "file") return props.path;
  const name = props.label.replace(/^[@$]/, "");
  if (props.kind === "plugin") {
    const key = activeSessionTab()?.threadId ?? NEW_CHAT_PLUGIN_KEY;
    const pluginId = props.path.startsWith("plugin://")
      ? props.path.slice("plugin://".length)
      : "";
    const plugin = store.threadPlugins[key]?.plugins.find(
      (p) => (pluginId && p.id === pluginId) || (!pluginId && p.name === name),
    );
    return plugin?.description || props.path;
  }
  const skill = store.skills.find((s) => s.name === name);
  return skill?.desc || props.path;
});

const kindLabel = computed(() =>
  props.kind === "file" ? "文件" : props.kind === "plugin" ? "插件" : "技能",
);

function onClick() {
  if (!clickable.value) return;
  openLink(props.path, sessionWorkspace());
}

// ---------------- 自定义悬浮卡片 ----------------
const tipId = `ref-tip-${Math.random().toString(36).slice(2, 9)}`;
const triggerEl = ref<HTMLElement | null>(null);
const tipEl = ref<HTMLElement | null>(null);
const open = ref(false);
const placed = ref(false);
const pos = ref({ top: 0, left: 0, arrowLeft: 0, below: false });
let showTimer: number | undefined;

function clearShowTimer() {
  if (showTimer) window.clearTimeout(showTimer);
  showTimer = undefined;
}

function show() {
  clearShowTimer();
  showTimer = window.setTimeout(() => {
    void openTooltip();
  }, props.delay);
}

async function openTooltip() {
  const el = triggerEl.value;
  if (!el) return;
  open.value = true;
  placed.value = false;
  await nextTick();
  const rect = el.getBoundingClientRect();
  const cardRect = tipEl.value?.getBoundingClientRect();
  const cardW = cardRect?.width || 240;
  const cardH = cardRect?.height || 64;
  const gap = 8;
  const below = rect.top < cardH + gap + 16;
  const top = below ? rect.bottom + gap : rect.top - cardH - gap;
  let left = rect.left + rect.width / 2 - cardW / 2;
  left = Math.max(8, Math.min(left, window.innerWidth - cardW - 8));
  const arrowLeft = rect.left + rect.width / 2 - left;
  pos.value = { top, left, arrowLeft, below };
  placed.value = true;
}

function hide() {
  clearShowTimer();
  open.value = false;
  placed.value = false;
}

onBeforeUnmount(() => {
  clearShowTimer();
});
</script>

<template>
  <button
    ref="triggerEl"
    v-if="clickable"
    type="button"
    class="mention-inline clickable"
    :aria-describedby="open ? tipId : undefined"
    @click="onClick"
    @mouseenter="show"
    @mouseleave="hide"
    @focus="show"
    @blur="hide"
    @keydown.esc="hide"
  >
    {{ props.label }}
  </button>
  <span
    ref="triggerEl"
    v-else
    class="mention-inline"
    :aria-describedby="open ? tipId : undefined"
    @mouseenter="show"
    @mouseleave="hide"
  >
    {{ props.label }}
  </span>

  <Teleport to="body">
    <div
      v-if="open"
      ref="tipEl"
      class="ref-tooltip"
      :class="{
        'ref-tooltip--placed': placed,
        'ref-tooltip--below': pos.below,
      }"
      role="tooltip"
      :style="{ top: pos.top + 'px', left: pos.left + 'px' }"
    >
      <span
        class="ref-tooltip-arrow"
        :style="{ left: pos.arrowLeft + 'px' }"
      ></span>
      <div class="ref-tooltip-name">
        {{ props.label }}
        <span class="ref-tooltip-kind">{{ kindLabel }}</span>
      </div>
      <div class="ref-tooltip-desc">{{ tipText }}</div>
    </div>
  </Teleport>
</template>
