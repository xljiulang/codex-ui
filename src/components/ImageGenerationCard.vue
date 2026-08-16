<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { assetUrl } from "../lib/asset";
import type { ImageGenerationItem } from "../lib/types";

const props = defineProps<{ item: ImageGenerationItem }>();

/** 从 result 中尽力提取可展示的图片地址（路径 / http(s) / data: URL） */
function extractResultSrc(result: unknown): string {
  if (typeof result === "string" && result.trim()) return result.trim();
  if (result && typeof result === "object") {
    const o = result as Record<string, unknown>;
    for (const k of ["path", "url", "src", "dataUrl", "data_url"]) {
      const v = o[k];
      if (typeof v === "string" && v.trim()) return v.trim();
    }
  }
  return "";
}

const src = computed(() => {
  const s = extractResultSrc(props.item.result);
  if (!s) return "";
  if (/^data:/i.test(s) || /^https?:\/\//i.test(s)) return s;
  // 本地绝对路径（含盘符或 / 开头）→ Tauri asset URL；其它形状原样展示
  if (s.includes(":") || s.startsWith("/")) return assetUrl(s);
  return s;
});
const hasImage = computed(() => !!src.value);

const statusLabel = computed(() => {
  const s = props.item.status;
  if (s === "in_progress" || s === "inProgress" || s === "pending" || s === "started")
    return "生成中";
  if (s === "completed" || s === "succeeded" || s === "done") return "完成";
  if (s === "failed" || s === "error") return "失败";
  return typeof s === "string" && s ? s : "";
});
const statusClass = computed(() => {
  if (statusLabel.value === "生成中") return "running";
  if (statusLabel.value === "完成") return "ok";
  if (statusLabel.value === "失败") return "fail";
  return "";
});

const rawJson = computed(() => JSON.stringify(props.item, null, 2));
const imgErr = ref(false);
const lightboxSrc = ref("");

function openLightbox() {
  if (hasImage.value) lightboxSrc.value = src.value;
}
function closeLightbox() {
  lightboxSrc.value = "";
}
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") closeLightbox();
}
watch(lightboxSrc, (v) => {
  if (v) window.addEventListener("keydown", onKey);
  else window.removeEventListener("keydown", onKey);
});
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="image-gen-card">
    <div v-if="statusLabel" class="tool-meta">
      <span class="status-dot" :class="statusClass"></span>
      <span>{{ statusLabel }}</span>
    </div>
    <div v-if="item.revisedPrompt" class="tool-meta">
      提示：{{ item.revisedPrompt }}
    </div>
    <img
      v-if="hasImage && !imgErr"
      class="image-gen-img clickable"
      :src="src"
      alt="生成图片"
      loading="lazy"
      decoding="async"
      @click="openLightbox()"
      @error="imgErr = true"
    />
    <div v-else-if="hasImage && imgErr" class="img-fallback">图片加载失败</div>
    <div v-else-if="!hasImage && statusLabel !== '生成中'" class="tool-json">
      {{ rawJson }}
    </div>
    <div
      v-if="lightboxSrc"
      class="lightbox"
      role="dialog"
      aria-label="图片预览"
      @click="closeLightbox"
    >
      <img :src="lightboxSrc" alt="图片预览" @click.stop />
    </div>
  </div>
</template>
