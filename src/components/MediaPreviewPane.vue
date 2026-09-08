<script setup lang="ts">
import { ref, watch } from "vue";
import type { PreviewEditorTab } from "../composables/useEditorTabs";

const props = defineProps<{ tab: PreviewEditorTab }>();

/** 媒体元素加载/解码失败（缺少解码器、格式不受支持、文件损坏等） */
const mediaError = ref(false);

// 切换预览标签或媒体地址变化（外部刷新）时复位错误态
watch(
  [() => props.tab, () => props.tab.mediaUrl],
  () => {
    mediaError.value = false;
  },
);
</script>

<template>
  <div v-if="mediaError" class="preview-note preview-error">
    无法播放该文件（可能缺少解码器或格式不受支持）
  </div>
  <div
    v-else-if="tab.previewType === 'video'"
    class="preview-media preview-media-video"
  >
    <video
      :src="tab.mediaUrl"
      controls
      preload="metadata"
      @error="mediaError = true"
    ></video>
  </div>
  <div v-else class="preview-media preview-media-audio">
    <audio
      :src="tab.mediaUrl"
      controls
      preload="metadata"
      @error="mediaError = true"
    ></audio>
  </div>
</template>
