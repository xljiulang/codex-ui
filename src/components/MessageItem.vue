<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import MarkdownText from "./MarkdownText.vue";
import ReasoningBlock from "./ReasoningBlock.vue";
import RefChip from "./RefChip.vue";
import ToolCard from "./ToolCard.vue";
import { formatDuration } from "../lib/format";
import { copyText } from "../lib/clipboard";
import {
  FILE_MENTION_HEADING,
  MY_REQUEST_MARKER,
  parseInlineMentions,
  parseFileMentionSection,
  type InlineSegment,
} from "../lib/mention";
import type { ThreadItem, UserInput } from "../lib/types";

const props = defineProps<{ item: ThreadItem }>();

function imageSrc(path: string): string {
  try {
    return convertFileSrc(path);
  } catch {
    return path;
  }
}

function partType(c: unknown): string {
  return ((c as UserInput)?.type) ?? "";
}

function partName(c: unknown): string {
  const u = c as UserInput;
  return u.type === "mention" || u.type === "skill" ? u.name : "";
}

function partRefPath(c: unknown): string {
  const u = c as UserInput;
  return u.type === "mention" || u.type === "skill" ? u.path : "";
}

/** 文本项正文：含 Files 段时取 `## My request:` 之后，否则整段 */
function bodyText(c: unknown): string {
  const u = c as UserInput;
  if (u.type !== "text") return "";
  const marker = `\n${MY_REQUEST_MARKER}\n`;
  const idx = u.text.indexOf(marker);
  if (idx >= 0 && u.text.includes(FILE_MENTION_HEADING)) {
    return u.text.slice(idx + marker.length);
  }
  return u.text;
}

/** 解析文本项正文中的内联引用片段（Files 段文件不在此，由附件区渲染） */
function partInline(c: unknown): InlineSegment[] {
  const u = c as UserInput;
  if (u.type !== "text") return [];
  return parseInlineMentions(bodyText(u));
}

/** 附件区文件：跨 text 项解析 Files 段、保序合并 */
const bubbleFiles = computed(() => {
  const out: { name: string; path: string }[] = [];
  for (const c of (props.item.content as unknown[]) ?? []) {
    const u = c as UserInput;
    if (u.type === "text") out.push(...parseFileMentionSection(u.text));
  }
  return out;
});

/** 附件区图片：localImage 项 */
const bubbleImages = computed(
  () =>
    ((props.item.content as unknown[]) ?? []).filter(
      (x) => (x as UserInput).type === "localImage",
    ) as Extract<UserInput, { type: "localImage" }>[],
);

function hasInlineRefs(c: unknown): boolean {
  return partInline(c).some((s) => s.type === "ref");
}

function segRefKind(
  seg: Extract<InlineSegment, { type: "ref" }>,
): "file" | "plugin" | "skill" {
  if (seg.prefix === "$") return "skill";
  return seg.path.startsWith("plugin://") ? "plugin" : "file";
}

/** 同一条消息里内联引用（[@name]/[$name]）的名称集合 */
function inlineRefNameSet(content: unknown): Set<string> {
  const names = new Set<string>();
  for (const x of (content as unknown[]) ?? []) {
    for (const s of partInline(x)) {
      if (s.type === "ref") names.add(s.name);
    }
  }
  return names;
}

function hasInlineRefNamed(content: unknown, name: string): boolean {
  return inlineRefNameSet(content).has(name);
}

/** 同名内联引用的前缀（@/$），用于结构化 skill 项的渲染前缀 */
function inlinePrefixFor(content: unknown, name: string): string | null {
  for (const x of (content as unknown[]) ?? []) {
    for (const s of partInline(x)) {
      if (s.type === "ref" && s.name === name) return s.prefix;
    }
  }
  return null;
}

/** 结构化 skill 项的前缀：同消息内存在同名插件链接（[@name]）时按插件渲染 @，否则按技能渲染 $ */
function skillPrefixFor(content: unknown, name: string): string {
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
  props.item.type === "todoList";

// ---------- 时间戳 ----------
function formatTime(ts: number): string {
  const d = new Date(ts);
  return `${String(d.getHours()).padStart(2, "0")}:${String(
    d.getMinutes(),
  ).padStart(2, "0")}`;
}
const time = computed(() =>
  typeof props.item.startedAtMs === "number"
    ? formatTime(props.item.startedAtMs as number)
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
  <div v-if="item.type === 'userMessage'" class="msg msg-user">
    <div class="bubble">
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
        />
        <template v-for="(img, k) in bubbleImages" :key="'img' + k">
          <img
            v-if="!attachmentImgErrors.has(k)"
            class="user-image attachment-image clickable"
            :src="imageSrc(img.path)"
            alt="图片"
            loading="lazy"
            decoding="async"
            @click="openLightbox(imageSrc(img.path))"
            @error="markAttachmentImgError(k)"
          />
          <div v-else class="img-fallback">图片加载失败</div>
        </template>
      </div>
        <template v-for="(c, i) in (item.content as unknown[]) ?? []" :key="i">
        <template v-if="partType(c) === 'localImage'">
          <!-- 图片已渲染到附件区 -->
        </template>
        <template v-else-if="partType(c) === 'text'">
          <template v-if="hasInlineRefs(c)">
            <template v-for="(seg, j) in partInline(c)" :key="'seg' + j">
              <RefChip
                v-if="seg.type === 'ref'"
                :path="seg.path"
                :label="seg.prefix + seg.name"
                :kind="segRefKind(seg)"
              />
              <span v-else-if="seg.text" class="md-inline">
                <MarkdownText :text="seg.text" />
              </span>
            </template>
          </template>
          <MarkdownText v-else :text="bodyText(c)" />
        </template>
        <template v-else-if="partType(c) === 'skill'">
          <RefChip
            v-if="!hasInlineRefNamed(item.content, partName(c))"
            :path="partRefPath(c)"
            :label="skillPrefixFor(item.content, partName(c)) + partName(c)"
            kind="skill"
          />
        </template>
        <RefChip
          v-else
          :path="partRefPath(c)"
          :label="'@' + partName(c)"
          kind="file"
        />
      </template>
      <span v-if="time" class="msg-time">{{ time }}</span>
    </div>
  </div>
  <div v-else-if="item.type === 'agentMessage' || item.type === 'plan'" class="msg msg-agent">
    <span v-if="item.phase === 'commentary' && item.streaming === true" class="phase-badge">进行中</span>
    <div
      v-if="isFinalAnswer && !item.streaming"
      class="msg-actions agent-actions"
    >
      <button
        class="msg-action-btn"
        :aria-label="'复制回复'"
        @click="copyAgentMessage()"
      >
        {{ copiedAgent ? "已复制" : "复制" }}
      </button>
    </div>
    <div v-if="isFinalAnswer && !item.streaming" class="agent-final">
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
    <div class="msg-error">{{ (item.message as string) ?? "发生错误" }}</div>
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
        :src="imageSrc(String(item.path ?? ''))"
        alt="图片"
        loading="lazy"
        decoding="async"
        @click="openLightbox(imageSrc(String(item.path ?? '')))"
        @error="markImgErr()"
      />
      <div v-else class="img-fallback">图片加载失败</div>
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
