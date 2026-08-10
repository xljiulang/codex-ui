<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import MarkdownText from "./MarkdownText.vue";
import ReasoningBlock from "./ReasoningBlock.vue";
import RefChip from "./RefChip.vue";
import ToolCard from "./ToolCard.vue";
import { formatDuration } from "../lib/format";
import {
  FILE_MENTION_HEADING,
  parseInlineMentions,
  parseFileMentionSection,
  parsePluginMentionLinks,
  parseSkillMentionLinks,
  stripMentionContext,
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

function partPath(c: unknown): string {
  const u = c as UserInput;
  return u.type === "localImage" ? u.path : "";
}

function partName(c: unknown): string {
  const u = c as UserInput;
  return u.type === "mention" || u.type === "skill" ? u.name : "";
}

function partRefPath(c: unknown): string {
  const u = c as UserInput;
  return u.type === "mention" || u.type === "skill" ? u.path : "";
}

function partFiles(c: unknown): { name: string; path: string }[] {
  const u = c as UserInput;
  return u.type === "text" ? parseFileMentionSection(u.text) : [];
}

function partSkillLinks(c: unknown): { name: string; path: string }[] {
  const u = c as UserInput;
  return u.type === "text" ? parseSkillMentionLinks(u.text) : [];
}

function partPluginLinks(c: unknown): { name: string; path: string }[] {
  const u = c as UserInput;
  return u.type === "text" ? parsePluginMentionLinks(u.text) : [];
}

/** 解析文本项中的内联引用片段；含旧 Files 段的历史消息视为 legacy，不走内联路径 */
function partInline(c: unknown): InlineSegment[] {
  const u = c as UserInput;
  if (u.type !== "text") return [];
  if (u.text.includes(FILE_MENTION_HEADING)) return [];
  return parseInlineMentions(u.text);
}

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

/** 同一条消息里结构化 skill 项的名称集合 */
function structuredSkillNames(content: unknown): Set<string> {
  const names = new Set<string>();
  for (const x of (content as unknown[]) ?? []) {
    const u = x as UserInput;
    if (u?.type === "skill") names.add(u.name);
  }
  return names;
}

/** 同一条消息里插件链接（[@name]）的名称集合 */
function pluginLinkNames(content: unknown): Set<string> {
  const names = new Set<string>();
  for (const x of (content as unknown[]) ?? []) {
    for (const p of partPluginLinks(x)) names.add(p.name);
  }
  return names;
}

/** 是否已有同名的结构化 skill 项（避免与文本链接重复渲染） */
function hasStructuredItemNamed(content: unknown, name: string): boolean {
  return structuredSkillNames(content).has(name);
}

/** 结构化 skill 项的前缀：同消息内存在同名插件链接（[@name]）时按插件渲染 @，否则按技能渲染 $ */
function skillPrefixFor(content: unknown, name: string): string {
  return (
    inlinePrefixFor(content, name) ??
    (pluginLinkNames(content).has(name) ? "@" : "$")
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
          <template v-if="hasInlineRefs(c)">
            <template
              v-for="(seg, j) in partInline(c)"
              :key="'seg' + j"
            >
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
          <template v-else>
            <RefChip
              v-for="(f, j) in partFiles(c)"
              :key="'f' + j"
              :path="f.path"
              :label="'@' + f.name"
              kind="file"
            />
            <template v-for="(p, j) in partPluginLinks(c)" :key="'p' + j">
              <RefChip
                v-if="!hasStructuredItemNamed(item.content, p.name)"
                :path="p.path"
                :label="'@' + p.name"
                kind="plugin"
              />
            </template>
            <template v-for="(s, k) in partSkillLinks(c)" :key="'s' + k">
              <RefChip
                v-if="!hasStructuredItemNamed(item.content, s.name)"
                :path="s.path"
                :label="'$' + s.name"
                kind="skill"
              />
            </template>
            <MarkdownText :text="partTextClean(c)" />
          </template>
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
