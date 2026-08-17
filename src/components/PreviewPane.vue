<script setup lang="ts">
import { computed, defineAsyncComponent, ref, watch } from "vue";
import { relPathOf } from "../lib/format";
import type { PreviewEditorTab } from "../composables/useEditorTabs";

const props = defineProps<{ tab: PreviewEditorTab }>();

// pdf.js 较重，懒加载避免拖累主窗口首屏
const PdfPreviewPane = defineAsyncComponent(() => import("./PdfPreviewPane.vue"));
const XlsxPreviewPane = defineAsyncComponent(
  () => import("./XlsxPreviewPane.vue"),
);

const relPath = computed(() => relPathOf(props.tab.workspace, props.tab.path));
const kindLabel = computed(() =>
  props.tab.previewType === "pdf"
    ? "PDF"
    : props.tab.previewType === "xlsx"
      ? "表格"
      : "图像",
);
const imgError = ref(false);
/** 图像缩放：1 表示适应窗口（CSS contain），其余按原始像素等比放大 */
const MIN_ZOOM = 0.25;
const MAX_ZOOM = 4;
const ZOOM_STEP = 1.25;
const zoom = ref(1);
const naturalW = ref(0);
const naturalH = ref(0);

const percentLabel = computed(() => `${Math.round(zoom.value * 100)}%`);
const imageStyle = computed(() => {
  if (zoom.value === 1 || !naturalW.value || !naturalH.value) return {};
  return {
    width: `${Math.round(naturalW.value * zoom.value)}px`,
    height: `${Math.round(naturalH.value * zoom.value)}px`,
  };
});

function onImgLoad(e: Event) {
  const img = e.target as HTMLImageElement;
  naturalW.value = img.naturalWidth || 0;
  naturalH.value = img.naturalHeight || 0;
}

function zoomIn() {
  zoom.value = Math.min(MAX_ZOOM, zoom.value * ZOOM_STEP);
}

function zoomOut() {
  zoom.value = Math.max(MIN_ZOOM, zoom.value / ZOOM_STEP);
}

function fitZoom() {
  zoom.value = 1;
}

// 切换预览标签或图片地址变化（外部刷新）时复位加载失败状态与缩放
watch(
  [() => props.tab, () => props.tab.imageUrl],
  () => {
    imgError.value = false;
    zoom.value = 1;
    naturalW.value = 0;
    naturalH.value = 0;
  },
);
</script>

<template>
  <div class="preview-pane">
    <div class="preview-head">
      <span class="preview-path">{{ relPath }}</span>
      <span class="preview-kind">{{ kindLabel }}</span>
    </div>
    <div v-if="tab.loading" class="preview-note">正在加载预览…</div>
    <div v-else-if="tab.error" class="preview-note preview-error">
      无法预览该文件（{{ tab.error }}）
    </div>
    <template v-else>
      <div v-if="tab.previewType === 'image'" class="preview-image">
        <div class="preview-zoom-toolbar">
          <button
            class="preview-zoom-btn"
            type="button"
            :disabled="zoom <= MIN_ZOOM"
            aria-label="缩小"
            @click="zoomOut()"
          >
            −
          </button>
          <span class="preview-zoom-percent">{{ percentLabel }}</span>
          <button
            class="preview-zoom-btn"
            type="button"
            :disabled="zoom >= MAX_ZOOM"
            aria-label="放大"
            @click="zoomIn()"
          >
            ＋
          </button>
          <button
            class="preview-zoom-btn"
            type="button"
            @click="fitZoom()"
          >
            适应窗口
          </button>
        </div>
        <div class="preview-image-stage">
          <img
            v-if="!imgError"
            :src="tab.imageUrl"
            alt=""
            draggable="false"
            :class="{ zoomed: zoom !== 1 }"
            :style="imageStyle"
            @load="onImgLoad"
            @error="imgError = true"
          />
          <div v-else class="preview-note preview-error">无法预览该图片</div>
        </div>
      </div>
      <PdfPreviewPane v-else-if="tab.previewType === 'pdf'" :tab="tab" />
      <XlsxPreviewPane v-else :tab="tab" />
    </template>
  </div>
</template>
