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
  FILE_MENTION_HEADING,
  MY_REQUEST_MARKER,
  parseInlineMentions,
  parseFileMentionSection,
  type InlineSegment,
} from "../lib/mention";
import { isUserInput, type ThreadItem, type UserInput } from "../lib/types";
import type { SessionTab } from "../composables/useCodex";

const props = defineProps<{ item: ThreadItem; tab: SessionTab }>();

/** 用户消息内容项：运行时过滤为合法 UserInput（协议外未知形状直接丢弃） */
const contentItems = computed(() => {
  const content = props.item.content;
  return Array.isArray(content) ? content.filter(isUserInput) : [];
});

/** 执行计划消息：文本项拼接后命中 PLEASE IMPLEMENT THIS PLAN: 前缀时卡片化展示 */
const EXECUTE_PLAN_PREFIX = "PLEASE IMPLEMENT THIS PLAN:";
const userText = computed(() =>
  contentItems.value.map((c) => bodyText(c)).join("\n"),
);
const isExecutePlan = computed(() =>
  userText.value.trimStart().toUpperCase().startsWith(EXECUTE_PLAN_PREFIX),
);
const executePlanText = computed(() => {
  const t = userText.value.trimStart();
  if (!t.toUpperCase().startsWith(EXECUTE_PLAN_PREFIX)) return "";
  return t.slice(EXECUTE_PLAN_PREFIX.length).trim();
});

/** 文本项正文：含 Files 段时取 `## My request:` 之后，否则整段 */
function bodyText(c: UserInput): string {
  if (c.type !== "text") return "";
  const marker = `\n${MY_REQUEST_MARKER}\n`;
  const idx = c.text.indexOf(marker);
  if (idx >= 0 && c.text.includes(FILE_MENTION_HEADING)) {
    return c.text.slice(idx + marker.length);
  }
  return c.text;
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
const attachmentImgErrors = ref(new Set<number>());
const imgErr = ref(false);
function openLightbox(src: string) {
  lightboxSrc.value = src;
}
function closeLightbox() {
  lightboxSrc.value = "";
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
    data-turn-anchor
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
            @click="openLightbox(assetUrl(img.path))"
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
        @click="openLightbox(assetUrl(String(item.path ?? '')))"
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
    <img :src="lightboxSrc" alt="图片预览" @click.stop />
  </div>
</template>
