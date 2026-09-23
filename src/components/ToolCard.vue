<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  isThreadItemType,
  type DynamicToolCallItem,
  type FileChangeItem,
  type ImageGenerationItem,
  type McpToolCallItem,
  type ThreadItem,
  type TodoListItem,
  type WebSearchItem,
} from "../lib/types";
import { useElapsed } from "../composables/useElapsed";
import { useThrottledRef } from "../composables/useThrottledRef";
import { useTailWindow } from "../composables/useTailWindow";
import { formatDuration, formatElapsed } from "../lib/format";
import { ansiToHtmlWithState, type AnsiStyle } from "../lib/ansi";
import { copyText } from "../lib/clipboard";
import {
  ICON_AGENTS,
  ICON_ARROW_DOWN,
  ICON_ARROW_RIGHT,
  ICON_CHECKLIST,
  ICON_EDIT,
  ICON_GLOBE,
  ICON_IMAGE,
  ICON_TERMINAL,
  ICON_TOOL,
} from "../lib/icons";
import FileChangeCard from "./FileChangeCard.vue";
import ImageGenerationCard from "./ImageGenerationCard.vue";
import TodoListCard from "./TodoListCard.vue";
import WebSearchCard from "./WebSearchCard.vue";

const props = defineProps<{ item: ThreadItem }>();
const expanded = ref(false);
const outputExpanded = ref(false);
const type = computed(() => props.item.type);

const running = computed(() => {
  const s = props.item.status;
  return (
    s === "in_progress" ||
    s === "inProgress" ||
    s === "pending" ||
    s === "started"
  );
});

const startedAt = computed(() =>
  typeof props.item.startedAtMs === "number"
    ? (props.item.startedAtMs as number)
    : Date.now(),
);
const { elapsed } = useElapsed(startedAt.value);

const durationMs = computed(() =>
  typeof props.item.durationMs === "number"
    ? (props.item.durationMs as number)
    : null,
);

const timeLabel = computed(() => {
  if (running.value) return formatElapsed(elapsed.value);
  if (durationMs.value != null)
    return `耗时 ${formatDuration(durationMs.value)}`;
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
  if (statusLabel.value === "已拒绝" || statusLabel.value === "已取消")
    return "declined";
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
    case "imageGeneration":
      return "生成图片";
    case "todoList":
      return "计划";
    default:
      return "工具";
  }
});

/** 头部图标：按工具类型映射，默认工具调用扳手 */
const icon = computed(() => {
  switch (type.value) {
    case "commandExecution":
      return ICON_TERMINAL;
    case "mcpToolCall":
    case "dynamicToolCall":
      return ICON_TOOL;
    case "collabAgentToolCall":
      return ICON_AGENTS;
    case "webSearch":
      return ICON_GLOBE;
    case "fileChange":
      return ICON_EDIT;
    case "imageGeneration":
      return ICON_IMAGE;
    case "todoList":
      return ICON_CHECKLIST;
    default:
      return ICON_TOOL;
  }
});

/** 命令执行的真实命令：优先使用 commandActions 中的简洁命令，回退到完整命令行 */
const commandActions = computed(
  () =>
    (props.item.commandActions as
      { command?: string; type?: string }[] | undefined) ?? [],
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
  if (type.value === "imageGeneration") {
    return String(props.item.revisedPrompt ?? "");
  }
  if (isThreadItemType<FileChangeItem>(props.item, "fileChange")) {
    if (!props.item.changes.length) return "";
    const first = props.item.changes[0].path;
    return props.item.changes.length > 1
      ? `${first} (等${props.item.changes.length}个)`
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
const { ref: shownOutput, flush: flushOutput } = useThrottledRef(
  cappedOutput,
  80,
);
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

const argsJson = computed(() => {
  const a = props.item.arguments;
  if (a === undefined || a === null) return "";
  // 空对象参数（如无入参工具）不展示，避免出现孤零零的 {}
  if (typeof a === "object" && Object.keys(a as object).length === 0) return "";
  try {
    return JSON.stringify(a, null, 2);
  } catch {
    return String(a);
  }
});

const resultText = computed(() => {
  // dynamicToolCall 的结果在 contentItems（{type:"inputText", text} 等），MCP 在 result.content
  const isDynamic = isThreadItemType<DynamicToolCallItem>(
    props.item,
    "dynamicToolCall",
  );
  const raw: unknown[] | null | undefined = isDynamic
    ? props.item.contentItems
    : (props.item.result as { content?: unknown[] } | null | undefined)
        ?.content;
  if (!raw || raw.length === 0) return "";
  return raw
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
    .filter((s) => s !== "")
    .join("\n");
});

const errorText = computed(() => {
  const e = props.item.error as { message?: string } | null | undefined;
  return e?.message ?? "";
});

const mcpProgressText = computed(() =>
  isThreadItemType<McpToolCallItem>(props.item, "mcpToolCall")
    ? (props.item.progressText ?? "")
    : "",
);
const mcpProgressPercent = computed(() =>
  isThreadItemType<McpToolCallItem>(props.item, "mcpToolCall") &&
  typeof props.item.progressPercent === "number"
    ? props.item.progressPercent
    : null,
);
</script>

<template>
  <div
    class="assistant-card"
    :class="{
      expanded,
    }"
  >
    <div class="assistant-card-header">
      <button
        type="button"
        class="assistant-card-toggle"
        :aria-expanded="expanded"
        @click="expanded = !expanded"
      >
        <svg
          class="assistant-card-arrow"
          viewBox="0 0 24 24"
          aria-hidden="true"
        >
          <path :d="expanded ? ICON_ARROW_DOWN : ICON_ARROW_RIGHT" />
        </svg>
        <svg class="assistant-card-icon" viewBox="0 0 24 24" aria-hidden="true">
          <path :d="icon" />
        </svg>
        <span class="assistant-card-title">{{ title }}</span>
        <span class="assistant-card-sub">{{ sub }}</span>
        <span class="assistant-card-status">
          <template v-if="statusLabel">
            <span class="status-dot" :class="statusClass"></span>
            <span>{{ statusLabel }}</span>
          </template>
          <span v-if="timeLabel">{{ timeLabel }}</span>
        </span>
      </button>
      <button
        v-if="commandText"
        type="button"
        class="copy-btn"
        :aria-label="'复制'"
        @click.stop="copyCommand()"
      >
        {{ copied ? "已复制" : "复制" }}
      </button>
    </div>

    <Transition name="assistant-card-body">
      <div v-if="expanded" class="assistant-card-body">
        <template v-if="type === 'commandExecution'">
          <div class="tool-command">
            <span class="tool-command-text">{{ commandText }}</span>
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

        <template
          v-else-if="type === 'mcpToolCall' || type === 'dynamicToolCall'"
        >
          <div v-if="mcpProgressText" class="tool-meta">
            进度：{{ mcpProgressText }}
          </div>
          <div v-if="mcpProgressPercent !== null" class="tool-progress">
            <div
              class="tool-progress-fill"
              :style="{
                width: Math.min(100, Math.max(0, mcpProgressPercent)) + '%',
              }"
            ></div>
          </div>
          <div v-if="argsJson" class="tool-json">{{ argsJson }}</div>
          <div v-if="resultText" class="tool-output">{{ resultText }}</div>
          <div v-if="errorText" class="tool-meta" style="color: var(--red)">
            {{ errorText }}
          </div>
        </template>

        <template v-else-if="type === 'collabAgentToolCall'">
          <div class="tool-meta">协作工具：{{ item.tool }}</div>
          <div v-if="item.prompt" class="tool-meta">
            提示：{{ item.prompt }}
          </div>
          <div v-if="item.model" class="tool-meta">模型：{{ item.model }}</div>
          <div
            v-if="item.agentsStates && Object.keys(item.agentsStates).length"
            class="tool-json"
          >
            {{ JSON.stringify(item.agentsStates, null, 2) }}
          </div>
        </template>

        <FileChangeCard
          v-else-if="item.type === 'fileChange'"
          :item="item as FileChangeItem"
          :expanded="expanded"
        />
        <TodoListCard
          v-else-if="item.type === 'todoList'"
          :item="item as TodoListItem"
        />
        <WebSearchCard
          v-else-if="item.type === 'webSearch'"
          :item="item as WebSearchItem"
        />
        <ImageGenerationCard
          v-else-if="item.type === 'imageGeneration'"
          :item="item as ImageGenerationItem"
        />
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.tool-command {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  font-family: var(--mono);
  font-size: var(--font-md);
  background: var(--console-bg-deep);
  color: var(--console-text);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: var(--space-3) var(--space-3);
  margin-bottom: var(--space-2);
  word-break: break-all;
  user-select: text;
}

.tool-command-text {
  flex: 1;
  min-width: 0;
  word-break: break-all;
}

.tool-output {
  font-family: var(--mono);
  font-size: var(--font-sm);
  background: var(--console-bg);
  color: var(--console-text);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: var(--space-3) var(--space-4);
  max-height: 220px;
  overflow: auto;
  white-space: pre-wrap;
  word-break: break-all;
  user-select: text;
}

.tool-output.collapsed {
  max-height: 72px;
}

.tool-output-cap {
  font-size: var(--font-sm);
  color: var(--text-faint);
  margin: var(--space-1) 0;
}

.tool-toggle {
  color: var(--blue);
  font-size: var(--font-sm);
  margin-top: var(--space-1);
  font-weight: 600;
  padding: 2px var(--space-1);
  border-radius: var(--radius-sm);
}

.tool-toggle:hover {
  color: var(--link-hover);
  background: rgba(var(--accent-rgb), 0.08);
}

.tool-progress {
  height: 4px;
  border-radius: 999px;
  background: var(--accent-soft);
  overflow: hidden;
  margin: var(--space-1) 0;
}

.tool-progress-fill {
  height: 100%;
  background: var(--accent);
  transition: width var(--ease-slow);
}
</style>
