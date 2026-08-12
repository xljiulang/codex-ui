<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import HistoryView from "./HistoryView.vue";
import ResourceView from "./ResourceView.vue";

/** 右侧面板默认/最小宽度（px） */
const DEFAULT_PANEL_WIDTH = 264;

const activeTab = ref<"history" | "resources">("history");
/** 面板宽度：仅本次运行生效，不持久化 */
const panelWidth = ref(DEFAULT_PANEL_WIDTH);
let resizeStartX = 0;
let resizeStartW = DEFAULT_PANEL_WIDTH;

/** 面板宽度钳制：最小为默认宽度，最大为半个窗口宽度 */
function clampPanelWidth(w: number) {
  const max = Math.max(DEFAULT_PANEL_WIDTH, Math.floor(window.innerWidth / 2));
  return Math.min(Math.max(w, DEFAULT_PANEL_WIDTH), max);
}

function startResize(e: PointerEvent) {
  e.preventDefault();
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
  window.removeEventListener("pointermove", onResizeMove);
  window.removeEventListener("pointerup", onResizeUp);
}

function onWindowResize() {
  panelWidth.value = clampPanelWidth(panelWidth.value);
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
  <aside class="history-panel" :style="{ width: panelWidth + 'px' }">
    <div
      class="history-resize-handle"
      aria-hidden="true"
      @pointerdown="startResize"
    ></div>
    <div class="panel-content">
      <HistoryView v-show="activeTab === 'history'" />
      <ResourceView
        v-show="activeTab === 'resources'"
        :active="activeTab === 'resources'"
      />
    </div>
    <div class="panel-tabs" role="tablist">
      <button
        class="panel-tab"
        :class="{ active: activeTab === 'history' }"
        role="tab"
        :aria-selected="activeTab === 'history'"
        @click="activeTab = 'history'"
      >
        历史会话
      </button>
      <button
        class="panel-tab"
        :class="{ active: activeTab === 'resources' }"
        role="tab"
        :aria-selected="activeTab === 'resources'"
        @click="activeTab = 'resources'"
      >
        会话资源
      </button>
    </div>
  </aside>
</template>
