<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref } from "vue";
import { localPathFromHref, openLink, sessionWorkspace } from "../lib/links";
import { openPathInApp } from "../composables/useSessionFs";
import type { SessionTab } from "../composables/useCodex";

const props = withDefaults(
  defineProps<{
    path: string;
    label: string;
    kind: "file" | "plugin" | "skill";
    tab: SessionTab;
    delay?: number;
  }>(),
  { delay: 120 },
);

/** 本地路径（非 plugin:// 等 URI）可点击；文件先应用内打开、失败回退资源管理器定位，技能直接定位 */
const clickable = computed(
  () => !!props.path && !props.path.startsWith("plugin://"),
);

/** 悬浮内容：文件显示路径；插件/技能显示说明（缓存缺失回退路径） */
const tipText = computed(() => {
  if (props.kind === "file") return props.path;
  const name = props.label.replace(/^[@$]/, "");
  if (props.kind === "plugin") {
    const pluginId = props.path.startsWith("plugin://")
      ? props.path.slice("plugin://".length)
      : "";
    const plugin = props.tab.plugins.plugins.find(
      (p) => (pluginId && p.id === pluginId) || (!pluginId && p.name === name),
    );
    return plugin?.description || props.path;
  }
  const skill = props.tab.skills.skills.find((s) => s.name === name);
  return skill?.desc || props.path;
});

const kindLabel = computed(() =>
  props.kind === "file" ? "文件" : props.kind === "plugin" ? "插件" : "技能",
);

async function onClick() {
  if (!clickable.value) return;
  if (props.kind === "file") {
    const cls = localPathFromHref(props.path, sessionWorkspace());
    if (cls?.kind === "local") {
      try {
        const opened = await openPathInApp(cls.path);
        if (opened) return;
      } catch {
        // 应用内打开异常：降级资源管理器定位，避免静默无反应
      }
    }
  }
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

<style scoped>
.mention-inline {
  display: inline-block;
  background: rgba(var(--accent-rgb), 0.18);
  border: 1px solid rgba(var(--accent-rgb), 0.24);
  border-radius: var(--radius-sm);
  padding: 1px var(--space-3);
  margin: 0 2px;
  font-size: var(--font-md);
}

.mention-inline.clickable {
  cursor: pointer;
}

.mention-inline.clickable:hover {
  background: rgba(var(--accent-rgb), 0.28);
}

.ref-tooltip {
  position: fixed;
  z-index: 100;
  max-width: 320px;
  padding: var(--space-3) var(--space-4);
  background: var(--float-bg);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius);
  box-shadow: var(--shadow-md);
  font-size: var(--font-md);
  color: var(--text-bright);
  opacity: 0;
  transform: translateY(4px);
  transition:
    opacity var(--ease),
    transform var(--ease);
  pointer-events: none;
  word-break: break-word;
}

.ref-tooltip--below {
  transform: translateY(-4px);
}

.ref-tooltip--placed {
  opacity: 1;
  transform: translateY(0);
}

.ref-tooltip-name {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-weight: 600;
  color: var(--text-bright);
}

.ref-tooltip-kind {
  font-size: var(--font-xs);
  font-weight: 400;
  color: var(--text-dim);
  background: rgba(var(--overlay-rgb), 0.08);
  border-radius: var(--radius-sm);
  padding: 1px var(--space-1);
}

.ref-tooltip-desc {
  margin-top: 3px;
  color: var(--text-dim);
  line-height: 1.5;
}

.ref-tooltip-arrow {
  position: absolute;
  bottom: -5px;
  width: 8px;
  height: 8px;
  transform: rotate(45deg);
  background: var(--bg-active);
  border-right: 1px solid var(--border-light);
  border-bottom: 1px solid var(--border-light);
}

.ref-tooltip--below .ref-tooltip-arrow {
  top: -5px;
  bottom: auto;
  border-right: none;
  border-bottom: none;
  border-left: 1px solid var(--border-light);
  border-top: 1px solid var(--border-light);
}
</style>
