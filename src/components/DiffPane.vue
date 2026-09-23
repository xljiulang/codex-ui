<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import hljs, { languageFromPath } from "../lib/highlight";
import { ICON_COPY, ICON_SUMMARY } from "../lib/icons";
import { diffKindLabel } from "../lib/gitChanges";
import {
  DIFF_MAX_RENDER_ROWS,
  canHighlightDiffRows,
  sliceDiffRows,
} from "../lib/diffView";
import ContextMenu from "./ContextMenu.vue";
import { useActionMenu } from "../composables/useActionMenu";
import { copyText } from "../lib/clipboard";
import type { DiffRow } from "../lib/types";
import type { DiffEditorTab } from "../composables/useEditorTabs";

const props = defineProps<{ tab: DiffEditorTab }>();

/** diff 正文滚动容器：右键菜单挂载点 + 整块复制来源 */
const bodyRef = ref<HTMLElement | null>(null);

// 自定义右键菜单（与文件编辑器同款脚手架）：仅提供「复制」
const {
  ctxMenu,
  openCtx,
  onWindowClick: onMenuWindowClick,
  onWindowScroll: onMenuWindowScroll,
  onKeydown: onMenuKeydown,
} = useActionMenu({ width: 150, scrollScope: ".diff-pane-body" });

const lang = computed(() => languageFromPath(props.tab.path));

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

/** 整份 diff 是否可以逐行高亮（未知语言、行数或文本量超预算时整块回退纯文本） */
const highlightEnabled = computed(() =>
  canHighlightDiffRows(!!lang.value, props.tab.rows),
);

/** 单行语法高亮；未启用或空行返回 null（模板回退纯文本渲染） */
function highlightLine(r: DiffRow): string | null {
  if (r.kind === "sep" || !r.text || !lang.value) return null;
  try {
    return hljs.highlight(r.text, {
      language: lang.value,
      ignoreIllegals: true,
    }).value;
  } catch {
    return null;
  }
}

type RenderedRow = DiffRow & { html?: string | null };

const renderedRows = computed<RenderedRow[]>(() =>
  highlightEnabled.value
    ? props.tab.rows.map((r) => ({ ...r, html: highlightLine(r) }))
    : props.tab.rows,
);

const brief = computed(() => props.tab.brief);

/** 简要模式：仅显示变更行与新旧分隔，隐藏未变化上下文 */
const filteredRows = computed(() =>
  brief.value
    ? renderedRows.value.filter((r) => r.kind !== "ctx")
    : renderedRows.value,
);

/** 实际渲染的行（超过上限截断）与提示所需的计数 */
const displayed = computed(() => sliceDiffRows(filteredRows.value));
const displayedRows = computed(() => displayed.value.visible);
const truncatedRows = computed(() => displayed.value.truncated);
const hiddenCtxRows = computed(() =>
  brief.value
    ? renderedRows.value.reduce(
        (count, r) => (r.kind === "ctx" ? count + 1 : count),
        0,
      )
    : 0,
);

/** 回退展示的原始 diff 行（同样受渲染上限保护） */
const fallbackDisplay = computed(() => sliceDiffRows(fallbackLines.value));
const fallbackRows = computed(() => fallbackDisplay.value.visible);
const truncatedFallbackRows = computed(() => fallbackDisplay.value.truncated);

function toggleBrief() {
  props.tab.brief = !props.tab.brief;
}

/** 取元素纯文本：剥除行号列与新旧分隔字样，仅保留正文 */
function plainText(el: HTMLElement): string {
  const clone = el.cloneNode(true) as HTMLElement;
  clone.querySelectorAll(".diff-no, .diff-sep-text").forEach((n) => n.remove());
  return (clone.textContent ?? "").trim();
}

/**
 * 复制目标文本：优先当前 DOM 选区；无选区则取光标所在行；
 * 都不成立时复制整个 diff 正文（逐行以换行连接），行号列一律排除。
 */
function copyTargetText(e: MouseEvent): string {
  try {
    const sel = window.getSelection();
    const selText = sel && !sel.isCollapsed ? sel.toString().trim() : "";
    if (selText) return selText;
  } catch {
    // 非浏览器环境（如单测）getSelection 异常时忽略选区
  }
  const target = e.target instanceof Element ? e.target : null;
  const row = target?.closest<HTMLElement>(".diff-row, .diff-line");
  if (row) return plainText(row);
  const body = bodyRef.value;
  if (!body) return "";
  const rows = body.querySelectorAll<HTMLElement>(".diff-row, .diff-line");
  return Array.from(rows)
    .map((r) => plainText(r))
    .join("\n")
    .trim();
}

function onDiffContext(e: MouseEvent) {
  const text = copyTargetText(e);
  openCtx(e, [
    { label: "复制", icon: ICON_COPY, action: () => void copyText(text) },
  ]);
}

function onWindowClick(e: MouseEvent) {
  // 右键菜单：任意外部 click 关闭（菜单内部点击由 ContextMenu @click.stop 处理）
  if (e.target instanceof Element && e.target.closest(".ctx-menu")) return;
  onMenuWindowClick();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  onMenuKeydown(e);
}

function onWindowScroll(e: Event) {
  // 右键菜单：仅 diff 正文自身滚动时关闭
  if (!(e.target instanceof Element)) return;
  if (!e.target.closest(".diff-pane-body")) return;
  onMenuWindowScroll(e);
}

onMounted(() => {
  window.addEventListener("click", onWindowClick);
  window.addEventListener("keydown", onKeydown);
  window.addEventListener("scroll", onWindowScroll, true);
});

onBeforeUnmount(() => {
  window.removeEventListener("click", onWindowClick);
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("scroll", onWindowScroll, true);
});
</script>

<template>
  <div class="diff-pane diff-embedded">
    <div class="diff-pane-head">
      <span class="diff-pane-title">
        <span class="diff-pane-path">{{ tab.path }}</span>
        <span class="change-kind" :class="tab.changeKind">
          {{ diffKindLabel(tab.changeKind) }}
        </span>
      </span>
      <span class="diff-pane-actions">
        <button
          v-if="!tab.loading && !tab.error && tab.rows.length"
          class="text-editor-icon-btn"
          :class="{ active: brief }"
          :aria-label="brief ? '完整显示' : '简要显示'"
          v-tooltip="brief ? '完整显示' : '简要显示'"
          @click="toggleBrief"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_SUMMARY" />
          </svg>
        </button>
      </span>
    </div>
    <div class="diff-pane-body" ref="bodyRef" @contextmenu="onDiffContext">
      <div v-if="tab.loading" class="diff-loading">正在加载文件内容…</div>
      <template v-else-if="tab.error || !tab.rows.length">
        <div class="diff-fallback-note">
          {{
            tab.error
              ? `无法预览该文件（${tab.error}），显示原始差异：`
              : "暂无差异内容"
          }}
        </div>
        <div v-if="truncatedFallbackRows" class="diff-fallback-note">
          差异过大：仅显示前 {{ DIFF_MAX_RENDER_ROWS }} 行，已省略
          {{ truncatedFallbackRows }} 行
        </div>
        <pre class="diff-view diff-preview">
          <div
            v-for="(l, j) in fallbackRows"
            :key="j"
            class="diff-line"
            :class="l.cls"
          >{{ l.text }}</div>
        </pre>
      </template>
      <template v-else>
        <div v-if="hiddenCtxRows" class="diff-fallback-note">
          行数较多，已折叠未变更的
          {{ hiddenCtxRows }} 行（右上角按钮可显示完整差异）
        </div>
        <div v-if="truncatedRows" class="diff-fallback-note">
          差异过大：仅渲染前 {{ DIFF_MAX_RENDER_ROWS }} 行，已省略
          {{ truncatedRows }} 行
        </div>
        <div class="diff-inline">
          <div
            v-for="(r, i) in displayedRows"
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
      </template>
    </div>

    <ContextMenu
      v-if="ctxMenu"
      :items="ctxMenu.items"
      :x="ctxMenu.x"
      :y="ctxMenu.y"
      @close="ctxMenu = null"
    />
  </div>
</template>
