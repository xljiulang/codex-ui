<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { call } from "../lib/ipc";
import type { ThreadItem } from "../lib/types";
import { useElapsed } from "../composables/useElapsed";
import { useThrottledRef } from "../composables/useThrottledRef";
import { useTailWindow } from "../composables/useTailWindow";
import { formatDuration, formatElapsed } from "../lib/format";
import { ansiToHtmlWithState, type AnsiStyle } from "../lib/ansi";
import { workspaceRoot } from "../lib/links";
import { copyText } from "../lib/clipboard";
import { openDiffTab } from "../composables/useEditorTabs";
import {
  diffKindLabel,
  normalizeDiffKind,
} from "../lib/gitChanges";

const props = defineProps<{ item: ThreadItem }>();
const expanded = ref(false);
const outputExpanded = ref(false);
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

const changes = computed(
  () =>
    (props.item.changes as
      | { path: string; kind: string | { type: string }; diff?: string }[]
      | undefined) ?? [],
);

const sub = computed(() => {
  if (type.value === "commandExecution") return commandText.value;
  if (type.value === "webSearch") {
    return String(props.item.query ?? "");
  }
  if (type.value === "fileChange") {
    if (!changes.value.length) return "";
    const first = changes.value[0].path;
    return changes.value.length > 1
      ? `${first} (等${changes.value.length}个)`
      : first;
  }
  return "";
});

const output = computed(() =>
  String(props.item.aggregatedOutput ?? props.item.output ?? ""),
);
// 渲染输入截断为最后 5000 行，避免超长输出流式期间 O(n^2)；状态里仍保留完整输出
const OUTPUT_LINE_CAP = 5000;
const { ref: cappedOutput, truncated: outputTruncated } = useTailWindow(
  output,
  OUTPUT_LINE_CAP,
  () => props.item,
);
const hasOutput = computed(() => cappedOutput.value.trim().length > 0);
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
  copied.value = await copyText(text);
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

const todos = computed(
  () =>
    (props.item.items as { text: string; completed: boolean }[] | undefined) ??
    [],
);

// 默认折叠：不再因运行中/失败/文件变更自动展开，点击头部才展开
const effectiveExpanded = computed(() => expanded.value);

function diffEntries(c: { kind: unknown; diff?: string }): { text: string; cls: string }[] {
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

// ---------- 文件变更：用条目自带的内联 diff 直接打开 ----------

function openPreview(c: { path: string; kind: unknown; diff?: string }) {
  if (!c.diff) return;
  void openDiffTab({
    path: c.path,
    kind: normalizeDiffKind(c.kind),
    diff: c.diff,
    workspace_root: workspaceRoot(),
  });
}
</script>

<template>
  <div
    class="tool-card"
    :class="{
      expanded: effectiveExpanded,
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
          :class="{ collapsed: !outputExpanded }"
        ></div>
        <div
          v-if="!outputExpanded && hasOutput"
          class="tool-toggle"
          @click="outputExpanded = true"
        >
          展开完整输出
        </div>
        <div
          v-if="outputExpanded && hasOutput"
          class="tool-toggle"
          @click="outputExpanded = false"
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
            @click.prevent="r.url ? void call('open_url', { url: r.url }) : undefined"
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
