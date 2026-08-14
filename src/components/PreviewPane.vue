<script setup lang="ts">
import { computed, defineAsyncComponent, ref, watch } from "vue";
import { relPathOf } from "../lib/format";
import type { PreviewEditorTab } from "../composables/useEditorTabs";

const props = defineProps<{ tab: PreviewEditorTab }>();

// pdf.js 较重，懒加载避免拖累主窗口首屏
const PdfPreviewPane = defineAsyncComponent(() => import("./PdfPreviewPane.vue"));

const relPath = computed(() => relPathOf(props.tab.root, props.tab.path));
const kindLabel = computed(() =>
  props.tab.previewType === "pdf" ? "PDF" : "图像",
);
const imgError = ref(false);

// 切换预览标签时复位图像加载失败状态
watch(
  () => props.tab,
  () => {
    imgError.value = false;
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
      <div v-if="tab.previewType === 'image'" class="preview-body preview-image">
        <img
          v-if="!imgError"
          :src="tab.imageUrl"
          alt=""
          draggable="false"
          @error="imgError = true"
        />
        <div v-else class="preview-note preview-error">无法预览该图片</div>
      </div>
      <PdfPreviewPane v-else :tab="tab" />
    </template>
  </div>
</template>
