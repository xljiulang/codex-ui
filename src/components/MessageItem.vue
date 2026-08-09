<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import MarkdownText from "./MarkdownText.vue";
import ReasoningBlock from "./ReasoningBlock.vue";
import ToolCard from "./ToolCard.vue";
import { formatDuration } from "../lib/format";
import {
  parseFileMentionSection,
  parseSkillMentionLinks,
  stripMentionContext,
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

function partPath(c: unknown): string {
  const u = c as UserInput;
  return u.type === "localImage" ? u.path : "";
}

function partName(c: unknown): string {
  const u = c as UserInput;
  return u.type === "mention" || u.type === "skill" ? u.name : "";
}

function partFiles(c: unknown): { name: string; path: string }[] {
  const u = c as UserInput;
  return u.type === "text" ? parseFileMentionSection(u.text) : [];
}

function partSkillLinks(c: unknown): { name: string; path: string }[] {
  const u = c as UserInput;
  return u.type === "text" ? parseSkillMentionLinks(u.text) : [];
}

/** 同一条消息里是否已有独立的结构化 skill 项（避免与文本链接重复渲染） */
function hasSkillItem(content: unknown): boolean {
  return ((content as unknown[]) ?? []).some(
    (x) => (x as UserInput)?.type === "skill",
  );
}

function partTextClean(c: unknown): string {
  const u = c as UserInput;
  return u.type === "text" ? stripMentionContext(u.text) : "";
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

// ---------- 复制 / 重试 ----------
async function copyText(t: string): Promise<boolean> {
  if (!t) return false;
  try {
    await navigator.clipboard.writeText(t);
    return true;
  } catch {
    try {
      const ta = document.createElement("textarea");
      ta.value = t;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand("copy");
      document.body.removeChild(ta);
      return ok;
    } catch {
      return false;
    }
  }
}

const copiedAgent = ref(false);
async function copyAgentMessage() {
  copiedAgent.value = await copyText(String(props.item.text ?? ""));
  window.setTimeout(() => (copiedAgent.value = false), 1500);
}

// ---------- 图片灯箱 / 加载失败占位 ----------
const lightboxSrc = ref("");
const imgErrors = ref(new Set<number>());
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
function markImgError(i: number) {
  const s = new Set(imgErrors.value);
  s.add(i);
  imgErrors.value = s;
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
      <template v-for="(c, i) in (item.content as unknown[]) ?? []" :key="i">
        <template v-if="partType(c) === 'localImage'">
          <img
            v-if="!imgErrors.has(i)"
            class="user-image clickable"
            :src="imageSrc(partPath(c))"
            alt="图片"
            loading="lazy"
            decoding="async"
            @click="openLightbox(imageSrc(partPath(c)))"
            @error="markImgError(i)"
          />
          <div v-else class="img-fallback">图片加载失败</div>
        </template>
        <template v-else-if="partType(c) === 'text'">
          <span
            v-for="(f, j) in partFiles(c)"
            :key="'f' + j"
            class="mention-inline"
            :title="f.path"
          >@{{ f.name }}</span>
          <span
            v-for="(s, k) in partSkillLinks(c)"
            v-if="!hasSkillItem(item.content)"
            :key="'s' + k"
            class="mention-inline"
            :title="s.path"
          >${{ s.name }}</span>
          <MarkdownText :text="partTextClean(c)" />
        </template>
        <span v-else-if="partType(c) === 'skill'" class="mention-inline">${{ partName(c) }}</span>
        <span v-else class="mention-inline">@{{ partName(c) }}</span>
      </template>
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
    <MarkdownText :text="String(item.text ?? '')" :streaming="item.streaming === true" />
    <span v-if="item.streaming === true" class="stream-cursor"></span>
    <span v-if="isFinalAnswer && time" class="msg-time">{{ time }}</span>
    <div v-if="memoryEntries(item.memoryCitation).length" class="memory-citation">
      <span class="memory-citation-title">记忆引用：</span>
      <span
        v-for="(e, i) in memoryEntries(item.memoryCitation)"
        :key="i"
        class="memory-citation-entry"
      >
        {{ e.path }}:{{ e.lineStart }}-{{ e.lineEnd
        }}{{ e.note ? `（${e.note}）` : "" }}
      </span>
    </div>
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
    <div class="context-compaction-note">上下文已压缩（长对话自动精简）</div>
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
