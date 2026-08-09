<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  applyReverseUnifiedDiff,
  buildInlineRows,
  type InlineRow,
} from "../lib/diff";

const props = defineProps<{
  path: string;
  kind: string;
  diff: string;
  workspaceRoot: string;
}>();
const emit = defineEmits<{ close: [] }>();

const loading = ref(true);
const error = ref("");
const rows = ref<InlineRow[]>([]);

function kindLabel(kind: string): string {
  if (kind === "add") return "新增";
  if (kind === "delete") return "删除";
  return "修改";
}

function resolvePath(p: string, root: string): string {
  const norm = (s: string) => s.replace(/\\/g, "/");
  let h = norm(p);
  if (/^[A-Za-z]:\//.test(h)) return h.replace(/\//g, "\\");
  if (/^\/[A-Za-z]:\//.test(h)) return h.slice(1).replace(/\//g, "\\");
  const base = norm(root || "").replace(/\/+$/, "");
  return `${base}/${h}`.replace(/\//g, "\\");
}

function fallbackLineCls(l: string): string {
  if (/^(\+\+\+|---)/.test(l)) return "diff-file";
  if (l.startsWith("+")) return "diff-add";
  if (l.startsWith("-")) return "diff-del";
  if (l.startsWith("@@")) return "diff-hunk";
  return "";
}

const fallbackLines = computed(() =>
  props.diff.split("\n").map((text) => ({
    text,
    cls: fallbackLineCls(text),
  })),
);

function rowOldNo(r: InlineRow): number | "" {
  return r.kind === "add" || r.kind === "sep" ? "" : r.oldNo;
}
function rowNewNo(r: InlineRow): number | "" {
  return r.kind === "del" || r.kind === "sep" ? "" : r.newNo;
}
function rowText(r: InlineRow): string {
  return r.kind === "sep" ? "" : r.text;
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const abs = resolvePath(props.path, props.workspaceRoot);
    let newContent = "";
    if (props.kind !== "delete") {
      newContent = await invoke<string>("read_file", { path: abs });
    }
    const oldContent = applyReverseUnifiedDiff(newContent, props.diff);
    rows.value = buildInlineRows(oldContent, newContent, props.diff);
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

onMounted(load);

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") emit("close");
}
onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div
    class="modal-mask diff-modal-mask"
    role="dialog"
    aria-label="文件差异预览"
    @click="emit('close')"
  >
    <div class="modal diff-modal" @click.stop>
      <div class="modal-head">
        <span class="modal-title diff-modal-title">
          <span class="change-kind" :class="kind">{{ kindLabel(kind) }}</span>
          <span class="diff-modal-path">{{ path }}</span>
        </span>
        <button
          class="modal-close"
          aria-label="关闭差异预览"
          @click="emit('close')"
        >
          ×
        </button>
      </div>
      <div class="modal-body diff-modal-body">
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
            v-for="(r, i) in rows"
            :key="i"
            class="diff-row"
            :class="r.kind"
          >
            <span class="diff-no old">{{ rowOldNo(r) }}</span>
            <span class="diff-no new">{{ rowNewNo(r) }}</span>
            <span v-if="r.kind === 'sep'" class="diff-sep-text">旧 | 新</span>
            <span v-else class="diff-text">{{ rowText(r) }}</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
