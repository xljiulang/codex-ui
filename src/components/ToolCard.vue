<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { ThreadItem } from "../lib/types";
import { useElapsed } from "../composables/useElapsed";
import { useThrottledRef } from "../composables/useThrottledRef";
import { formatDuration, formatElapsed } from "../lib/format";
import { ansiToHtmlWithState, type AnsiStyle } from "../lib/ansi";
import { workspaceRoot } from "../lib/links";

const props = defineProps<{ item: ThreadItem }>();
const expanded = ref(false);
const type = computed(() => props.item.type);

const running = computed(() => {
  const s = props.item.status;
  return s === "in_progress" || s === "inProgress" || s === "pending" || s === "started";
});

const startedAt = computed(() =>
  typeof props.item.startedAtMs === "number"
    ? (props.item.startedAtMs as number)
    : Date.now(),
);
const { elapsed } = useElapsed(startedAt.value);

const durationMs = computed(() =>
  typeof props.item.durationMs === "number" ? (props.item.durationMs as number) : null,
);

const timeLabel = computed(() => {
  if (running.value) return formatElapsed(elapsed.value);
  if (durationMs.value != null) return `耗时 ${formatDuration(durationMs.value)}`;
  return "";
});

const statusLabel = computed(() => {
  const s = props.item.status;
  if (running.value) return "进行中";
  if (s === "completed" || s === "succeeded") return "成功";
  if (s === "failed" || s === "error") return "失败";
  if (s === "declined") return "已拒绝";
  if (s === "canceled" || s === "cancelled") return "已取消";
  if (s === "interrupted") return "已中断";
  return typeof s === "string" ? s : "";
});

const statusClass = computed(() => {
  if (running.value) return "running";
  if (statusLabel.value === "成功") return "ok";
  if (statusLabel.value === "失败") return "fail";
  if (statusLabel.value === "已拒绝" || statusLabel.value === "已取消") return "declined";
  return "";
});

const title = computed(() => {
  switch (type.value) {
    case "commandExecution":
      return "执行命令";
    case "mcpToolCall":
      return `调用工具 ${String(props.item.server ?? "")}::${String(props.item.tool ?? "")}`;
    case "dynamicToolCall":
      return `调用工具 ${props.item.namespace ? `${String(props.item.namespace)}/` : ""}${String(
        props.item.tool ?? "",
      )}`;
    case "collabAgentToolCall":
      return "子代理协作";
    case "webSearch":
      return "搜索网络";
    case "fileChange":
      return "文件变更";
    case "todoList":
      return "计划";
    default:
      return "工具";
  }
});

/** 命令执行的真实命令：优先使用 commandActions 中的简洁命令，回退到完整命令行 */
const commandActions = computed(
  () =>
    (props.item.commandActions as
      | { command?: string; type?: string }[]
      | undefined) ?? [],
);

const commandText = computed(() => {
  const clean = commandActions.value
    .map((a) => String(a.command ?? ""))
    .filter(Boolean)
    .join(" ");
  if (clean) return clean;
  return String(props.item.command ?? "");
});

const sub = computed(() => {
  if (type.value === "commandExecution") return commandText.value;
  if (type.value === "webSearch") {
    return String(props.item.query ?? "");
  }
  return "";
});

const output = computed(() =>
  String(props.item.aggregatedOutput ?? props.item.output ?? ""),
);
const hasOutput = computed(() => output.value.trim().length > 0);
// 渲染输入截断为最后 5000 行，避免超长输出流式期间 O(n^2)；状态里仍保留完整输出
const OUTPUT_LINE_CAP = 5000;
const outputTruncated = computed(
  () => output.value.split("\n").length > OUTPUT_LINE_CAP,
);
const cappedOutput = computed(() => {
  const lines = output.value.split("\n");
  if (lines.length <= OUTPUT_LINE_CAP) return output.value;
  return lines.slice(-OUTPUT_LINE_CAP).join("\n");
});
const { ref: shownOutput, flush: flushOutput } = useThrottledRef(cappedOutput, 80);
watch(
  () => props.item.status,
  () => {
    if (!running.value) flushOutput();
  },
);
// ---------- 命令输出增量渲染 ----------
const outputRoot = ref<HTMLElement | null>(null);
let renderedLen = 0;
let ansiState: AnsiStyle = {};
let committedPrefix = "";
let lastTruncated = false;

function renderOutput(t: string) {
  const el = outputRoot.value;
  if (!el) {
    renderedLen = 0;
    ansiState = {};
    committedPrefix = "";
    return;
  }
  const truncated = outputTruncated.value;
  if (!t.startsWith(committedPrefix) || truncated !== lastTruncated) {
    // 条目替换/文本回退/截断切换：重置并整段重渲染
    renderedLen = 0;
    ansiState = {};
    committedPrefix = "";
    el.innerHTML = "";
  }
  lastTruncated = truncated;
  const delta = t.slice(renderedLen);
  let cut = delta.length;
  const esc = delta.lastIndexOf("\x1b");
  // 尾部转义序列不完整时暂缓渲染，等下一段补齐
  if (esc >= 0 && !/^\[[0-9;?]*[a-zA-Z]$/.test(delta.slice(esc))) cut = esc;
  const safe = delta.slice(0, cut);
  renderedLen += safe.length;
  committedPrefix = t.slice(0, renderedLen);
  if (safe) {
    const { html, style } = ansiToHtmlWithState(ansiState, safe);
    ansiState = style;
    el.insertAdjacentHTML("beforeend", html);
  }
}

watch(shownOutput, (t) => renderOutput(t), { immediate: true });
// 输出容器随 v-if 挂载/展开而出现，挂载后补一次渲染
watch(outputRoot, (el) => {
  if (el) renderOutput(shownOutput.value);
});

const copied = ref(false);
async function copyCommand() {
  const text = commandText.value;
  if (!text) return;
  let ok = false;
  try {
    await navigator.clipboard.writeText(text);
    ok = true;
  } catch {
    try {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      ok = document.execCommand("copy");
      document.body.removeChild(ta);
    } catch {
      ok = false;
    }
  }
  copied.value = ok;
  window.setTimeout(() => {
    copied.value = false;
  }, 1500);
}

/** webSearch 的结构化结果（协议为透明 JSON，尽力提取常见字段） */
const webResults = computed(() => {
  const r = props.item.results as unknown[] | null | undefined;
  if (!Array.isArray(r)) return [];
  return r.map((x) => {
    if (x && typeof x === "object") {
      const o = x as Record<string, unknown>;
      return {
        title: String(o.title ?? o.name ?? ""),
        url: String(o.url ?? o.link ?? ""),
        snippet: String(o.snippet ?? o.description ?? o.content ?? ""),
      };
    }
    return { title: String(x), url: "", snippet: "" };
  });
});
const webResultsUsable = computed(() =>
  webResults.value.some((r) => r.title || r.url),
);

const argsJson = computed(() => {
  const a = props.item.arguments;
  if (a === undefined || a === null) return "";
  try {
    return JSON.stringify(a, null, 2);
  } catch {
    return String(a);
  }
});

const resultText = computed(() => {
  const r = props.item.result as { content?: unknown[] } | null | undefined;
  if (!r?.content) return "";
  return r.content
    .map((c) => {
      if (c && typeof c === "object" && "text" in c) {
        return String((c as { text: string }).text);
      }
      if (typeof c === "string") return c;
      try {
        return JSON.stringify(c);
      } catch {
        return "";
      }
    })
    .join("\n");
});

const errorText = computed(() => {
  const e = props.item.error as { message?: string } | null | undefined;
  return e?.message ?? "";
});

const changes = computed(
  () =>
    (props.item.changes as
      | { path: string; kind: string | { type: string }; diff?: string }[]
      | undefined) ?? [],
);

const todos = computed(
  () =>
    (props.item.items as { text: string; completed: boolean }[] | undefined) ??
    [],
);

const effectiveExpanded = computed(
  () =>
    expanded.value ||
    type.value === "fileChange" ||
    running.value ||
    ((statusLabel.value === "失败" || statusLabel.value === "已拒绝") &&
      hasOutput.value),
);

function kindOf(kind: unknown): string {
  if (typeof kind === "string") return kind;
  if (kind && typeof kind === "object" && "type" in (kind as Record<string, unknown>)) {
    return String((kind as { type: unknown }).type);
  }
  return "update";
}

function kindLabel(kind: unknown): string {
  const k = kindOf(kind);
  if (k === "add") return "新增";
  if (k === "delete") return "删除";
  return "修改";
}

function diffEntries(c: { kind: unknown; diff?: string }): { text: string; cls: string }[] {
  const raw = c.diff ?? "";
  const lines = raw.split("\n");
  const kind = kindOf(c.kind);
  const hasMarkers = lines.some(
    (l) => /^[+-]/.test(l) && !/^(\+\+\+|---)/.test(l),
  );
  if (!hasMarkers) {
    const prefix = kind === "add" ? "+" : kind === "delete" ? "-" : "";
    return lines.map((l) => ({
      text: l ? `${prefix}${l}` : l,
      cls:
        kind === "add"
          ? "diff-add"
          : kind === "delete"
            ? "diff-del"
            : "",
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

// ---------- 文件变更：打开独立 diff 窗口 ----------
function openPreview(c: { path: string; kind: unknown; diff?: string }) {
  if (!c.diff) return;
  void invoke("open_diff_window", {
    params: {
      path: c.path,
      kind: kindOf(c.kind),
      diff: c.diff,
      workspace_root: workspaceRoot(),
    },
  }).catch(() => undefined);
}
</script>

<template>
  <div
    class="tool-card"
    :class="{
      expanded: effectiveExpanded,
      'tool-card--minimal':
        type === 'commandExecution' || type === 'fileChange',
    }"
  >
    <div class="tool-card-header" @click="expanded = !expanded">
      <span class="tool-card-title">{{ title }}</span>
      <span class="tool-card-sub">{{ sub }}</span>
      <span class="tool-card-status">
        <template v-if="statusLabel">
          <span class="status-dot" :class="statusClass"></span>
          <span>{{ statusLabel }}</span>
        </template>
        <span v-if="timeLabel">{{ timeLabel }}</span>
      </span>
    </div>

    <Transition name="card-body">
      <div v-if="effectiveExpanded" class="tool-card-body">
        <template v-if="type === 'commandExecution'">
        <div class="tool-command">
          <span class="tool-command-text">{{ commandText }}</span>
          <button
            v-if="commandText"
            class="copy-btn"
            :aria-label="'复制命令'"
            @click.stop="copyCommand()"
          >
            {{ copied ? "已复制" : "复制命令" }}
          </button>
        </div>
        <div v-if="item.cwd" class="tool-meta">工作目录：{{ item.cwd }}</div>
        <div v-if="outputTruncated" class="tool-output-cap">
          输出过长，仅显示末尾 {{ OUTPUT_LINE_CAP }} 行
        </div>
        <div
          v-if="hasOutput"
          ref="outputRoot"
          class="tool-output"
          :class="{ collapsed: !expanded }"
        ></div>
        <div
          v-if="!expanded && hasOutput"
          class="tool-toggle"
          @click="expanded = true"
        >
          展开完整输出
        </div>
        <div
          v-if="expanded && hasOutput"
          class="tool-toggle"
          @click="expanded = false"
        >
          收起输出
        </div>
        <div
          v-if="item.exitCode != null && item.status === 'failed'"
          class="tool-meta"
          style="color: var(--red)"
        >
          退出码：{{ item.exitCode }}
        </div>
      </template>

      <template v-else-if="type === 'mcpToolCall' || type === 'dynamicToolCall'">
        <div v-if="argsJson" class="tool-json">{{ argsJson }}</div>
        <div v-if="resultText" class="tool-output">{{ resultText }}</div>
        <div v-if="errorText" class="tool-meta" style="color: var(--red)">
          {{ errorText }}
        </div>
      </template>

      <template v-else-if="type === 'collabAgentToolCall'">
        <div class="tool-meta">协作工具：{{ item.tool }}</div>
        <div v-if="item.prompt" class="tool-meta">提示：{{ item.prompt }}</div>
        <div v-if="item.model" class="tool-meta">模型：{{ item.model }}</div>
        <div
          v-if="item.agentsStates && Object.keys(item.agentsStates).length"
          class="tool-json"
        >
          {{ JSON.stringify(item.agentsStates, null, 2) }}
        </div>
      </template>

      <template v-else-if="type === 'fileChange'">
        <div v-for="c in changes" :key="c.path" class="change-block">
          <div
            class="change-row"
            :class="{ clickable: !!c.diff }"
            :title="c.diff ? '点击查看完整差异' : ''"
            @click="openPreview(c)"
          >
            <span class="change-kind" :class="kindOf(c.kind)">{{ kindLabel(c.kind) }}</span>
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

      <template v-else-if="type === 'todoList'">
        <div class="change-list">
          <div v-for="t in todos" :key="t.text" class="todo-row" :class="{ done: t.completed }">
            <span>{{ t.completed ? "☑" : "☐" }}</span>
            <span>{{ t.text }}</span>
          </div>
        </div>
      </template>

      <template v-else-if="type === 'webSearch'">
        <div class="tool-meta">查询：{{ item.query }}</div>
        <div v-if="webResultsUsable" class="web-results">
          <a
            v-for="(r, i) in webResults"
            :key="i"
            class="web-result"
            :href="r.url || '#'"
            @click.prevent="r.url ? void invoke('open_url', { url: r.url }) : undefined"
          >
            <span class="web-result-title">{{ r.title || r.url || "（无标题）" }}</span>
            <span v-if="r.snippet" class="web-result-snippet">{{ r.snippet }}</span>
          </a>
        </div>
        <div v-else-if="webResults.length" class="tool-json">
          {{ JSON.stringify(item.results) }}
        </div>
        </template>
      </div>
    </Transition>
  </div>

</template>
