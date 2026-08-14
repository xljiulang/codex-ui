<script setup lang="ts">
import { computed } from "vue";
import hljs, { languageFromPath } from "../lib/highlight";
import type { DiffRow } from "../lib/types";
import type { DiffEditorTab } from "../composables/useEditorTabs";

const props = defineProps<{ tab: DiffEditorTab }>();

const MAX_HIGHLIGHT_LINES = 20_000;
const lang = computed(() => languageFromPath(props.tab.path));

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
  (props.tab.fallback || "")
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
  if (props.tab.rows.length > MAX_HIGHLIGHT_LINES) return null;
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
  props.tab.rows.map((r) => ({ ...r, html: highlightLine(r) })),
);
</script>

<template>
  <div class="diff-window diff-embedded">
    <div class="diff-window-head">
      <span class="diff-window-title">
        <span class="diff-window-path">{{ tab.path }}</span>
        <span class="change-kind" :class="tab.changeKind">
          {{ kindLabel(tab.changeKind) }}
        </span>
      </span>
    </div>
    <div class="diff-window-body">
      <div v-if="tab.loading" class="diff-loading">正在加载文件内容…</div>
      <template v-else-if="tab.error || !tab.rows.length">
        <div class="diff-fallback-note">
          {{
            tab.error
              ? `无法预览该文件（${tab.error}），显示原始差异：`
              : "暂无差异内容"
          }}
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
