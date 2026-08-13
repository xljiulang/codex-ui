<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import hljs, { languageFromPath } from "../lib/highlight";

interface TextPreviewParams {
  root: string;
  path: string;
}

interface PreviewLine {
  text: string;
  html: string | null;
}

const loading = ref(true);
const error = ref("");
const params = ref<TextPreviewParams | null>(null);
const allLines = ref<string[]>([]);

const MAX_HIGHLIGHT_LINES = 20_000;
const lang = computed(() =>
  params.value ? languageFromPath(params.value.path) : null,
);

/** 逐行语法高亮：未知语言/空行/超大文件回退纯文本 */
function highlightLine(text: string): string | null {
  if (!text || !lang.value) return null;
  if (allLines.value.length > MAX_HIGHLIGHT_LINES) return null;
  try {
    return hljs.highlight(text, {
      language: lang.value,
      ignoreIllegals: true,
    }).value;
  } catch {
    return null;
  }
}

const lines = computed<PreviewLine[]>(() =>
  allLines.value.map((text) => ({ text, html: highlightLine(text) })),
);

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const p = await invoke<TextPreviewParams | null>(
      "take_text_preview_params",
    );
    if (!p) {
      error.value = "未找到文本预览参数";
      return;
    }
    params.value = p;
    const content = await invoke<string>("session_fs_read", {
      root: p.root,
      path: p.path,
    });
    allLines.value = content ? content.split("\n") : [];
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  void load();
  // 右键无菜单：仅阻止默认（避免“刷新”），不显示任何菜单
  window.addEventListener("contextmenu", onContextMenu);
});

onBeforeUnmount(() => {
  window.removeEventListener("contextmenu", onContextMenu);
});

function onContextMenu(e: Event) {
  e.preventDefault();
}
</script>

<template>
  <div class="text-preview-window">
    <div class="text-preview-head">
      <span class="text-preview-title">
        <span class="text-preview-path">{{ params?.path ?? "" }}</span>
        <span v-if="params" class="text-preview-lang">{{ lang ?? "text" }}</span>
      </span>
    </div>
    <div class="text-preview-body">
      <div v-if="loading" class="text-preview-loading">正在加载文件内容…</div>
      <div v-else-if="error" class="text-preview-note">
        无法预览该文件（{{ error }}）
      </div>
      <div v-else-if="!lines.length" class="text-preview-note">（空文件）</div>
      <div v-else class="text-preview-view">
        <div
          v-for="(l, i) in lines"
          :key="i"
          class="text-preview-line"
        >
          <span class="text-preview-no">{{ i + 1 }}</span>
          <span class="text-preview-text">
            <span v-if="l.html" v-html="l.html"></span>
            <template v-else>{{ l.text }}</template>
          </span>
        </div>
      </div>
    </div>
  </div>
</template>
