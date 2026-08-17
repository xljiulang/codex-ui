<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import GitView from "./GitView.vue";
import HistoryView from "./HistoryView.vue";
import ResourceView from "./ResourceView.vue";
import { store, type PanelTab } from "../composables/useCodex";
import { ICON_GIT } from "../lib/icons";

/** 面板初始宽度 = 窗口宽度 × 比例；最小宽度 = 比例值与固定兜底取较大者 */
const PANEL_WIDTH_RATIO = 0.24;
const MIN_PANEL_WIDTH_RATIO = 0.16;
const MIN_PANEL_WIDTH_PX = 200;

/** 激活 Tab：全局 store 状态，新建会话入口可统一切回资源 */
const activeTab = computed<PanelTab>(() => store.panelTab);
/** 面板宽度：仅本次运行生效，不持久化 */
const panelWidth = ref(initialPanelWidth());
const isDragging = ref(false);
let resizeStartX = 0;
let resizeStartW = initialPanelWidth();

/** Tab 顺序：会话为第一个/默认 tab；供方向键切换使用 */
const TAB_ORDER = ["history", "resources", "git"] as const;

/** 面板初始宽度：窗口宽度的 24%（1280 下约 307px，保持现状比例） */
function initialPanelWidth() {
  return Math.round(window.innerWidth * PANEL_WIDTH_RATIO);
}

/** 面板最小宽度：窗口宽度的 16% 与固定兜底 200px 取较大者 */
function minPanelWidth() {
  return Math.max(
    Math.round(window.innerWidth * MIN_PANEL_WIDTH_RATIO),
    MIN_PANEL_WIDTH_PX,
  );
}

/** 面板宽度钳制：最小为比例+兜底，最大为半个窗口宽度 */
function clampPanelWidth(w: number) {
  const max = Math.max(initialPanelWidth(), Math.floor(window.innerWidth / 2));
  return Math.min(Math.max(w, minPanelWidth()), max);
}

function startResize(e: PointerEvent) {
  e.preventDefault();
  isDragging.value = true;
  resizeStartX = e.clientX;
  resizeStartW = panelWidth.value;
  window.addEventListener("pointermove", onResizeMove);
  window.addEventListener("pointerup", onResizeUp);
}

function onResizeMove(e: PointerEvent) {
  // 面板在右侧：向左拖（X 减小）使面板变宽
  panelWidth.value = clampPanelWidth(resizeStartW + (resizeStartX - e.clientX));
}

function onResizeUp() {
  isDragging.value = false;
  window.removeEventListener("pointermove", onResizeMove);
  window.removeEventListener("pointerup", onResizeUp);
}

function onWindowResize() {
  panelWidth.value = clampPanelWidth(panelWidth.value);
}

/** Tab 栏方向键切换：左右箭头循环移动焦点 */
function moveTab(e: KeyboardEvent) {
  const idx = TAB_ORDER.indexOf(activeTab.value);
  let next = idx;
  if (e.key === "ArrowRight") {
    next = (idx + 1) % TAB_ORDER.length;
  } else if (e.key === "ArrowLeft") {
    next = (idx - 1 + TAB_ORDER.length) % TAB_ORDER.length;
  } else {
    return;
  }
  e.preventDefault();
  store.panelTab = TAB_ORDER[next];
}

onMounted(() => {
  window.addEventListener("resize", onWindowResize);
});
onBeforeUnmount(() => {
  window.removeEventListener("resize", onWindowResize);
  onResizeUp();
});
</script>

<template>
  <aside class="right-panel" :style="{ width: panelWidth + 'px' }">
    <div
      class="right-panel-resize-handle"
      :class="{ dragging: isDragging }"
      aria-hidden="true"
      @pointerdown="startResize"
    ></div>
    <div class="panel-content">
      <HistoryView v-show="activeTab === 'history'" />
      <ResourceView
        v-show="activeTab === 'resources'"
        :active="activeTab === 'resources'"
      />
      <GitView v-show="activeTab === 'git'" :active="activeTab === 'git'" />
    </div>
    <div class="panel-tabs" role="tablist" @keydown="moveTab">
      <button
        class="panel-tab"
        :class="{ active: activeTab === 'history' }"
        role="tab"
        :tabindex="activeTab === 'history' ? 0 : -1"
        :aria-selected="activeTab === 'history'"
        @click="store.panelTab = 'history'"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M13 3c-4.97 0-9 4.03-9 9H1l3.89 3.89.07.14L9 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42C8.27 19.99 10.51 21 13 21c4.97 0 9-4.03 9-9s-4.03-9-9-9zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z"
          />
        </svg>
        <span>会话</span>
      </button>
      <button
        class="panel-tab"
        :class="{ active: activeTab === 'resources' }"
        role="tab"
        :tabindex="activeTab === 'resources' ? 0 : -1"
        :aria-selected="activeTab === 'resources'"
        @click="store.panelTab = 'resources'"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M20 6h-8l-2-2H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V8h16v10z"
          />
        </svg>
        <span>资源</span>
      </button>
      <button
        class="panel-tab"
        :class="{ active: activeTab === 'git' }"
        role="tab"
        :tabindex="activeTab === 'git' ? 0 : -1"
        :aria-selected="activeTab === 'git'"
        @click="store.panelTab = 'git'"
      >
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path :d="ICON_GIT" />
        </svg>
        <span>Git</span>
      </button>
    </div>
  </aside>
</template>
