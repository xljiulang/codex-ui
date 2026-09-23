<script setup lang="ts">
import { computed } from "vue";
import { openDiffTab } from "../composables/useEditorTabs";
import { diffKindLabel, normalizeDiffKind } from "../lib/gitChanges";
import { sessionWorkspace } from "../lib/links";
import type { FileChangeItem } from "../lib/types";

const props = defineProps<{
  item: FileChangeItem;
  /** 卡片展开态（控制内联 diff 的折叠） */
  expanded: boolean;
}>();

const changes = computed(() => props.item.changes ?? []);

function diffEntries(c: {
  kind: unknown;
  diff?: string;
}): { text: string; cls: string }[] {
  const raw = c.diff ?? "";
  const lines = raw.split("\n");
  const kind = normalizeDiffKind(c.kind);
  const hasMarkers = lines.some(
    (l) => /^[+-]/.test(l) && !/^(\+\+\+|---)/.test(l),
  );
  if (!hasMarkers) {
    const prefix = kind === "add" ? "+" : kind === "delete" ? "-" : "";
    return lines.map((l) => ({
      text: l ? `${prefix}${l}` : l,
      cls: kind === "add" ? "diff-add" : kind === "delete" ? "diff-del" : "",
    }));
  }
  return lines.map((l) => {
    if (/^(\+\+\+|---)/.test(l)) return { text: l, cls: "diff-file" };
    if (l.startsWith("+")) return { text: l, cls: "diff-add" };
    if (l.startsWith("-")) return { text: l, cls: "diff-del" };
    if (l.startsWith("@@")) return { text: l, cls: "diff-hunk" };
    return { text: l, cls: "" };
  });
}

/** 用条目自带的内联 diff 直接打开 */
function openPreview(c: { path: string; kind: unknown; diff?: string }) {
  if (!c.diff) return;
  void openDiffTab({
    path: c.path,
    kind: normalizeDiffKind(c.kind),
    diff: c.diff,
    workspace: sessionWorkspace(),
  });
}
</script>

<template>
  <div v-for="c in changes" :key="c.path" class="change-block">
    <div
      class="change-row"
      :class="{ clickable: !!c.diff }"
      v-tooltip="c.diff ? '点击查看完整差异' : ''"
      @click="openPreview(c)"
    >
      <span class="change-kind" :class="normalizeDiffKind(c.kind)">
        {{ diffKindLabel(normalizeDiffKind(c.kind)) }}
      </span>
      <span>{{ c.path }}</span>
    </div>
    <div v-if="c.diff" class="diff-wrap">
      <pre class="diff-view" :class="{ collapsed: !expanded }">
        <div
          v-for="(l, j) in diffEntries(c)"
          :key="j"
          class="diff-line"
          :class="l.cls"
        >{{ l.text }}</div>
      </pre>
    </div>
  </div>
  <div v-if="!changes.length" class="tool-meta">暂无变更</div>
</template>
