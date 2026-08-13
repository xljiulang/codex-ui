<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import hljs, { languageFromPath } from "../lib/highlight";
import type { DiffRow } from "../lib/types";
import { pathBaseName } from "../lib/format";

interface DiffPreviewParams {
  path: string;
  kind: string;
  diff: string;
  workspace_root: string;
}

const loading = ref(true);
const error = ref("");
const rows = ref<DiffRow[]>([]);
const params = ref<DiffPreviewParams | null>(null);

const MAX_HIGHLIGHT_LINES = 20_000;
const lang = computed(() =>
  params.value ? languageFromPath(params.value.path) : null,
);

function kindLabel(kind: string): string {
  if (kind === "add") return "新增";
  if (kind === "delete") return "删除";
  return "修改";
}

function fallbackLineCls(l: string): string {
  if (/^(\+\+\+|---)/.test(l)) return "diff-file";
  if (l.startsWith("+")) return "diff-add";
  if (l.startsWith("-")) return "diff-del";
  if (l.startsWith("@@")) return "diff-hunk";
  return "";
}

const fallbackLines = computed(() =>
  (params.value?.diff ?? "")
    .split("\n")
    .map((text) => ({ text, cls: fallbackLineCls(text) })),
);

function rowOldNo(r: DiffRow): number | "" {
  return r.kind === "add" || r.kind === "sep" ? "" : r.oldNo;
}
function rowNewNo(r: DiffRow): number | "" {
  return r.kind === "del" || r.kind === "sep" ? "" : r.newNo;
}
function rowText(r: DiffRow): string {
  return r.kind === "sep" ? "" : r.text;
}

/** 逐行语法高亮：未知语言/空行/超大文件回退纯文本 */
function highlightLine(r: DiffRow): string | null {
  if (r.kind === "sep" || !r.text || !lang.value) return null;
  if (rows.value.length > MAX_HIGHLIGHT_LINES) return null;
  try {
    return hljs.highlight(r.text, {
      language: lang.value,
      ignoreIllegals: true,
    }).value;
  } catch {
    return null;
  }
}

const renderedRows = computed(() =>
  rows.value.map((r) => ({ ...r, html: highlightLine(r) })),
);

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const p = await invoke<DiffPreviewParams | null>("take_diff_params");
    if (!p) {
      error.value = "未找到 diff 参数";
      return;
    }
    params.value = p;
    // 窗口标题栏显示文件名（不含路径）
    try {
      await getCurrentWindow().setTitle(pathBaseName(p.path));
    } catch {
      // 非 Tauri 环境（单测/浏览器）忽略
    }
    rows.value = await invoke<DiffRow[]>("build_diff_preview", { params: p });
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
  <div class="diff-window">
    <div class="diff-window-head">
      <span class="diff-window-title">
        <span class="change-kind" :class="params?.kind ?? ''">
          {{ kindLabel(params?.kind ?? "") }}
        </span>
        <span class="diff-window-path">{{ params?.path ?? "" }}</span>
      </span>
    </div>
    <div class="diff-window-body">
      <div v-if="loading" class="diff-loading">正在加载文件内容…</div>
      <template v-else-if="error || !rows.length">
        <div class="diff-fallback-note">
          {{ error ? `无法预览该文件（${error}），显示原始差异：` : "暂无差异内容" }}
        </div>
        <pre class="diff-view diff-preview">
          <div
            v-for="(l, j) in fallbackLines"
            :key="j"
            class="diff-line"
            :class="l.cls"
          >{{ l.text }}</div>
        </pre>
      </template>
      <div v-else class="diff-inline">
        <div
          v-for="(r, i) in renderedRows"
          :key="i"
          class="diff-row"
          :class="r.kind"
        >
          <span class="diff-no old">{{ rowOldNo(r) }}</span>
          <span class="diff-no new">{{ rowNewNo(r) }}</span>
          <span v-if="r.kind === 'sep'" class="diff-sep-text">旧 | 新</span>
          <span v-else class="diff-text">
            <span v-if="r.html" v-html="r.html"></span>
            <template v-else>{{ rowText(r) }}</template>
          </span>
        </div>
      </div>
    </div>
  </div>
</template>
