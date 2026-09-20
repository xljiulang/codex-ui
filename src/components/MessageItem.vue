<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import MarkdownText from "./MarkdownText.vue";
import PlanTextCard from "./PlanTextCard.vue";
import ReasoningBlock from "./ReasoningBlock.vue";
import RefChip from "./RefChip.vue";
import ToolCard from "./ToolCard.vue";
import { assetUrl } from "../lib/asset";
import { formatChatTime, formatDuration } from "../lib/format";
import { copyText } from "../lib/clipboard";
import {
  parseInlineMentions,
  parseFileMentionSection,
  type InlineSegment,
} from "../lib/mention";
import { summarizeUserMessage, textItemBody } from "../lib/userMessage";
import {
  isThreadItemType,
  isUserInput,
  type ThreadItem,
  type UserInput,
  type UserMessageItem,
} from "../lib/types";
import type { SessionTab } from "../composables/useCodex";

const props = defineProps<{ item: ThreadItem; tab: SessionTab }>();

/** 用户消息内容项：运行时过滤为合法 UserInput（协议外未知形状直接丢弃） */
const contentItems = computed(() => {
  const content = props.item.content;
  return Array.isArray(content) ? content.filter(isUserInput) : [];
});

/**
 * 用户消息摘要：优先用入库时持久化的 derived，缺省（旧数据/直接构造）即时回退计算，
 * 保证气泡复制/执行计划识别与回合导航消费同一份解析结果。
 */
const summary = computed(() => {
  const it = props.item;
  if (!isThreadItemType<UserMessageItem>(it, "userMessage")) {
    return summarizeUserMessage(undefined);
  }
  return it.derived ?? summarizeUserMessage(it.content);
});
const userText = computed(() => summary.value.text);
const isExecutePlan = computed(() => summary.value.isExecutePlan);
const executePlanText = computed(() => summary.value.executePlanText);

/** 文本项正文：含 Files 段时取 `## My request:` 之后，否则整段（与摘要解析同源） */
function bodyText(c: UserInput): string {
  return c.type === "text" ? textItemBody(c.text) : "";
}

/** 解析文本项正文中的内联引用片段（Files 段文件不在此，由附件区渲染） */
function partInline(c: UserInput): InlineSegment[] {
  if (c.type !== "text") return [];
  return parseInlineMentions(bodyText(c));
}

/** 附件区文件：跨 text 项解析 Files 段、保序合并 */
const bubbleFiles = computed(() => {
  const out: { name: string; path: string }[] = [];
  for (const c of contentItems.value) {
    if (c.type === "text") out.push(...parseFileMentionSection(c.text));
  }
  return out;
});

/** 附件区图片：localImage 项 */
const bubbleImages = computed(
  () =>
    contentItems.value.filter(
      (x): x is Extract<UserInput, { type: "localImage" }> =>
        x.type === "localImage",
    ),
);

function hasInlineRefs(c: UserInput): boolean {
  return partInline(c).some((s) => s.type === "ref");
}

function segRefKind(
  seg: Extract<InlineSegment, { type: "ref" }>,
): "file" | "plugin" | "skill" {
  if (seg.prefix === "$") return "skill";
  return seg.path.startsWith("plugin://") ? "plugin" : "file";
}

/** 同一条消息里内联引用（[@name]/[$name]）的名称集合 */
function inlineRefNameSet(content: UserInput[]): Set<string> {
  const names = new Set<string>();
  for (const x of content) {
    for (const s of partInline(x)) {
      if (s.type === "ref") names.add(s.name);
    }
  }
  return names;
}

function hasInlineRefNamed(content: UserInput[], name: string): boolean {
  return inlineRefNameSet(content).has(name);
}

/** 同名内联引用的前缀（@/$），用于结构化 skill 项的渲染前缀 */
function inlinePrefixFor(content: UserInput[], name: string): string | null {
  for (const x of content) {
    for (const s of partInline(x)) {
      if (s.type === "ref" && s.name === name) return s.prefix;
    }
  }
  return null;
}

/** 结构化 skill 项的前缀：同消息内存在同名插件链接（[@name]）时按插件渲染 @，否则按技能渲染 $ */
function skillPrefixFor(content: UserInput[], name: string): string {
  return inlinePrefixFor(content, name) ?? "$";
}

function memoryEntries(
  c: unknown,
): { path: string; lineStart: number; lineEnd: number; note: string }[] {
  const m = c as { entries?: unknown[] } | null | undefined;
  if (!m?.entries || !Array.isArray(m.entries)) return [];
  return m.entries as {
    path: string;
    lineStart: number;
    lineEnd: number;
    note: string;
  }[];
}

const isTool =
  props.item.type === "commandExecution" ||
  props.item.type === "mcpToolCall" ||
  props.item.type === "dynamicToolCall" ||
  props.item.type === "collabAgentToolCall" ||
  props.item.type === "webSearch" ||
  props.item.type === "fileChange" ||
  props.item.type === "imageGeneration" ||
  props.item.type === "todoList";

const time = computed(() =>
  typeof props.item.startedAtMs === "number"
    ? formatChatTime(props.item.startedAtMs as number)
    : "",
);
const isFinalAnswer = computed(() => props.item.phase === "final_answer");
const memoryList = computed(() => memoryEntries(props.item.memoryCitation));

// ---------- 复制 / 重试 ----------

const copiedAgent = ref(false);
async function copyAgentMessage() {
  copiedAgent.value = await copyText(String(props.item.text ?? ""));
  window.setTimeout(() => (copiedAgent.value = false), 1500);
}

const copiedUser = ref(false);
async function copyUserMessage() {
  copiedUser.value = await copyText(userText.value);
  window.setTimeout(() => (copiedUser.value = false), 1500);
}

// ---------- 图片灯箱 / 加载失败占位 ----------
const lightboxSrc = ref("");
/** 灯箱图片的可复制来源（本地绝对路径）——右键「复制图像」用，asset URL 不能直接读文件 */
const lightboxSource = ref("");
const attachmentImgErrors = ref(new Set<number>());
const imgErr = ref(false);
function openLightbox(src: string, source = "") {
  lightboxSrc.value = src;
  lightboxSource.value = source;
}
function closeLightbox() {
  lightboxSrc.value = "";
  lightboxSource.value = "";
}
function onLightboxKey(e: KeyboardEvent) {
  if (e.key === "Escape") closeLightbox();
}
function markAttachmentImgError(k: number) {
  const s = new Set(attachmentImgErrors.value);
  s.add(k);
  attachmentImgErrors.value = s;
}
function markImgErr() {
  imgErr.value = true;
}
watch(lightboxSrc, (v) => {
  if (v) window.addEventListener("keydown", onLightboxKey);
  else window.removeEventListener("keydown", onLightboxKey);
});
onBeforeUnmount(() => window.removeEventListener("keydown", onLightboxKey));

// ---------- 未知类型兜底 ----------
const showRaw = ref(false);
const rawJson = computed(() => JSON.stringify(props.item, null, 2));
</script>

<template>
  <div
    v-if="item.type === 'userMessage'"
    class="msg msg-user"
    :data-turn-anchor="item.id"
  >
    <div class="bubble">
      <button
        v-if="userText"
        type="button"
        class="copy-btn"
        :aria-label="'复制'"
        @click="copyUserMessage()"
      >
        {{ copiedUser ? "已复制" : "复制" }}
      </button>
      <div
        v-if="bubbleFiles.length || bubbleImages.length"
        class="bubble-attachments"
      >
        <RefChip
          v-for="(f, j) in bubbleFiles"
          :key="'fa' + j"
          :path="f.path"
          :label="'@' + f.name"
          kind="file"
          :tab="props.tab"
        />
        <template v-for="(img, k) in bubbleImages" :key="'img' + k">
          <img
            v-if="!attachmentImgErrors.has(k)"
            class="user-image attachment-image clickable"
            :src="assetUrl(img.path)"
            alt="图片"
            loading="lazy"
            decoding="async"
            @click="openLightbox(assetUrl(img.path), img.path)"
            @error="markAttachmentImgError(k)"
          />
          <div v-else class="img-fallback">图片加载失败</div>
        </template>
      </div>
      <PlanTextCard
        v-if="isExecutePlan"
        :plan-text="executePlanText"
      />
      <template v-for="(c, i) in contentItems" :key="i">
        <template v-if="c.type === 'localImage'">
          <!-- 图片已渲染到附件区 -->
        </template>
        <template v-else-if="c.type === 'text' && !isExecutePlan">
          <template v-if="hasInlineRefs(c)">
            <template v-for="(seg, j) in partInline(c)" :key="'seg' + j">
              <RefChip
                v-if="seg.type === 'ref'"
                :path="seg.path"
                :label="seg.prefix + seg.name"
                :kind="segRefKind(seg)"
                :tab="props.tab"
              />
              <span v-else-if="seg.text" class="md-inline">
                <MarkdownText :text="seg.text" />
              </span>
            </template>
          </template>
          <MarkdownText v-else :text="bodyText(c)" />
        </template>
        <template v-else-if="c.type === 'skill'">
          <RefChip
            v-if="!hasInlineRefNamed(contentItems, c.name)"
            :path="c.path"
            :label="skillPrefixFor(contentItems, c.name) + c.name"
            kind="skill"
            :tab="props.tab"
          />
        </template>
        <RefChip
          v-else-if="c.type === 'mention'"
          :path="c.path"
          :label="'@' + c.name"
          kind="file"
          :tab="props.tab"
        />
      </template>
      <span v-if="time" class="msg-time">{{ time }}</span>
    </div>
  </div>
  <div v-else-if="item.type === 'plan'" class="msg">
    <!-- 助理端计划是计划模式产出，默认展开直接可见 -->
    <PlanTextCard :plan-text="String(item.text ?? '')" default-open />
  </div>
  <div v-else-if="item.type === 'agentMessage'" class="msg msg-agent">
    <span v-if="item.phase === 'commentary' && item.streaming === true" class="phase-badge">进行中</span>
    <div v-if="isFinalAnswer && !item.streaming" class="agent-final">
      <button
        type="button"
        class="copy-btn"
        :aria-label="'复制'"
        @click="copyAgentMessage()"
      >
        {{ copiedAgent ? "已复制" : "复制" }}
      </button>
      <MarkdownText :text="String(item.text ?? '')" />
      <span v-if="time" class="msg-time">{{ time }}</span>
      <div v-if="memoryList.length" class="memory-citation">
        <span class="memory-citation-title">记忆引用：</span>
        <span
          v-for="(e, i) in memoryList"
          :key="i"
          class="memory-citation-entry"
        >
          {{ e.path }}:{{ e.lineStart }}-{{ e.lineEnd
          }}{{ e.note ? `（${e.note}）` : "" }}
        </span>
      </div>
    </div>
    <template v-else>
      <MarkdownText :text="String(item.text ?? '')" :streaming="item.streaming === true" />
      <span v-if="item.streaming === true" class="stream-cursor"></span>
      <div v-if="memoryList.length" class="memory-citation">
        <span class="memory-citation-title">记忆引用：</span>
        <span
          v-for="(e, i) in memoryList"
          :key="i"
          class="memory-citation-entry"
        >
          {{ e.path }}:{{ e.lineStart }}-{{ e.lineEnd
          }}{{ e.note ? `（${e.note}）` : "" }}
        </span>
      </div>
    </template>
  </div>
  <div v-else-if="item.type === 'reasoning'" class="msg">
    <ReasoningBlock :item="item" />
  </div>
  <div v-else-if="item.type === 'error'" class="msg">
    <div class="msg-error">{{ item.message ?? "发生错误" }}</div>
  </div>
  <div v-else-if="isTool" class="msg">
    <ToolCard :item="item" />
  </div>
  <div v-else-if="item.type === 'contextCompaction'" class="msg">
    <div class="context-compaction-note">上下文已压缩（长会话自动精简）</div>
  </div>
  <div v-else-if="item.type === 'imageView'" class="msg msg-user">
    <div class="bubble">
      <img
        v-if="!imgErr"
        class="user-image clickable"
        :src="assetUrl(String(item.path ?? ''))"
        alt="图片"
        loading="lazy"
        decoding="async"
        @click="openLightbox(assetUrl(String(item.path ?? '')), String(item.path ?? ''))"
        @error="markImgErr()"
      />
      <div v-else class="img-fallback">图片加载失败</div>
    </div>
  </div>
  <div v-else-if="item.type === 'subAgentActivity'" class="msg">
    <div class="sub-agent-note">
      子代理活动：{{ item.kind
      }}<template v-if="item.agentPath">（{{ item.agentPath }}）</template>
    </div>
  </div>
  <div v-else-if="item.type === 'sleep'" class="msg">
    <div class="unknown-item">等待 {{ formatDuration(Number(item.durationMs ?? 0)) }}</div>
  </div>
  <div v-else class="msg">
    <div class="unknown-item">
      暂不支持显示的项目类型：{{ item.type }}
      <button class="unknown-toggle" @click="showRaw = !showRaw">
        {{ showRaw ? "收起原始数据" : "查看原始数据" }}
      </button>
    </div>
    <pre v-if="showRaw" class="unknown-raw">{{ rawJson }}</pre>
  </div>

  <div
    v-if="lightboxSrc"
    class="lightbox"
    role="dialog"
    aria-label="图片预览"
    @click="closeLightbox"
  >
    <!-- 不加 @click.stop：点图片也冒泡到遮罩，一次点击即关闭；data-copy-source 供右键「复制图像」 -->
    <img
      :src="lightboxSrc"
      :data-copy-source="lightboxSource"
      alt="图片预览"
    />
  </div>
</template>

<style scoped>
/* 用户消息 */
.msg-user {
  display: flex;
  justify-content: flex-end;
}

.msg-user .bubble {
  max-width: 86%;
  background: rgba(var(--accent-rgb), 0.1);
  border: 1px solid rgba(var(--accent-rgb), 0.28);
  color: var(--text-bright);
  box-shadow: var(--inset-shadow), 0 2px 12px rgba(var(--accent-rgb), 0.13);
  padding: var(--space-3) var(--space-5);
  border-radius: var(--radius-lg) var(--radius-lg) var(--radius-sm) var(--radius-lg);
  white-space: pre-wrap;
  word-break: break-word;
  user-select: text;
  position: relative;
}

.msg-user .bubble .user-image {
  display: block;
  max-width: 100%;
  max-height: 320px;
  border-radius: var(--radius-sm);
  margin: var(--space-1) 0;
}

.msg-user .bubble .bubble-attachments {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin-bottom: var(--space-2);
}

.msg-user .bubble .bubble-attachments .attachment-image {
  width: 68px;
  height: 68px;
  object-fit: cover;
  border-radius: var(--radius);
  border: 1px solid var(--border);
  cursor: pointer;
  transition: border-color var(--ease);
}

.msg-user .bubble .bubble-attachments .attachment-image:hover {
  border-color: var(--accent-dim);
}

.msg-user .bubble .md-inline {
  display: inline-block;
  vertical-align: baseline;
  max-width: 100%;
}

.msg-user .bubble .md-inline :deep(.md > :last-child) {
  margin-bottom: 0;
}

/* 助手最终答复卡片：与工具卡同语言，左侧强调条突出“这一轮的回答” */
.agent-final {
  border: 1px solid var(--border);
  border-left: 3px solid var(--accent-dim);
  border-radius: var(--radius-lg);
  background: rgba(var(--accent-rgb), 0.04);
  padding: var(--space-5) var(--space-6);
}

.msg-time {
  display: block;
  margin-top: var(--space-2);
  font-size: var(--font-xs);
  color: var(--text-faint);
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.msg-agent .msg-time {
  text-align: left;
}

.msg-agent .agent-final {
  position: relative;
}
.msg-agent .copy-btn {
  position: absolute;
  top: 8px;
  right: 8px;
  opacity: 0;
}
.msg-agent:hover .copy-btn,
.msg-agent:focus-within .copy-btn {
  opacity: 1;
}
.msg-user .copy-btn {
  position: absolute;
  top: 8px;
  right: 8px;
  opacity: 0;
}
.msg-user .bubble:hover .copy-btn,
.msg-user:focus-within .copy-btn {
  opacity: 1;
}

/* 用户气泡内的 Markdown（布局适配，配色沿用共享 .md） */
.msg-user .bubble :deep(.md) {
  white-space: normal;
  line-height: 1.5;
}
.msg-user .bubble :deep(.md p) {
  margin-bottom: var(--space-2);
}
.msg-user .bubble :deep(.md > div > :last-child),
.msg-user .bubble :deep(.md > :last-child) {
  margin-bottom: 0;
}
.msg-user .bubble :deep(.md pre) {
  margin: var(--space-2) 0;
}

.msg-error {
  color: var(--red);
  font-size: var(--font-md);
  background: rgba(var(--red-rgb), 0.08);
  border: 1px solid rgba(var(--red-rgb), 0.3);
  border-left: 3px solid var(--red);
  padding: var(--space-3) var(--space-5);
  border-radius: var(--radius);
  line-height: 1.5;
}

.phase-badge {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--font-sm);
  font-weight: 600;
  color: var(--accent);
  background: var(--accent-soft);
  border: 1px solid rgba(var(--accent-rgb), 0.3);
  border-radius: 999px;
  padding: 2px var(--space-3);
  margin-bottom: var(--space-1);
}

.phase-badge::before {
  content: "";
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--accent);
  animation: dot-blink 1.1s ease-in-out infinite;
}

.stream-cursor {
  display: inline-block;
  width: 8px;
  height: 14px;
  margin-left: 2px;
  vertical-align: -2px;
  background: var(--accent);
  border-radius: var(--radius-sm);
  animation: blink 1s steps(2, start) infinite;
}

@keyframes blink {
  to {
    visibility: hidden;
  }
}

.memory-citation {
  font-size: var(--font-sm);
  color: var(--text-faint);
  margin-top: var(--space-2);
  line-height: 1.6;
}

.memory-citation-entry {
  display: block;
}

.context-compaction-note,
.unknown-item {
  text-align: center;
  color: var(--text-faint);
  font-size: var(--font-sm);
  padding: var(--space-2) 0;
}

/* 未知类型兜底 */
.unknown-toggle {
  margin-left: var(--space-3);
  font-size: var(--font-sm);
  color: var(--text-dim);
  background: none;
  border: 1px solid var(--border-light);
  border-radius: var(--radius-sm);
  padding: 2px var(--space-2);
  cursor: pointer;
}
.unknown-toggle:hover {
  color: var(--text-bright);
}
.unknown-raw {
  max-height: 240px;
  overflow: auto;
  font-size: var(--font-sm);
  color: var(--console-text);
  background: var(--console-bg);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: var(--space-3) var(--space-4);
  margin: var(--space-2) 0;
  white-space: pre-wrap;
  word-break: break-all;
}

/* 子代理活动提示（subAgentActivity） */
.sub-agent-note {
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: var(--space-1) var(--space-4);
  font-size: var(--font-md);
  color: var(--text-dim);
  background: var(--reasoning-bg);
  margin-bottom: var(--space-3);
}
</style>
