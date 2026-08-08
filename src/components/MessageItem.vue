<script setup lang="ts">
import { convertFileSrc } from "@tauri-apps/api/core";
import MarkdownText from "./MarkdownText.vue";
import ReasoningBlock from "./ReasoningBlock.vue";
import ToolCard from "./ToolCard.vue";
import { formatDuration } from "../lib/format";
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

function partText(c: unknown): string {
  const u = c as UserInput;
  return u.type === "text" ? u.text : "";
}

function partPath(c: unknown): string {
  const u = c as UserInput;
  return u.type === "localImage" ? u.path : "";
}

function partName(c: unknown): string {
  const u = c as UserInput;
  return u.type === "mention" || u.type === "skill" ? u.name : "";
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
</script>

<template>
  <div v-if="item.type === 'userMessage'" class="msg msg-user">
    <div class="bubble">
      <template v-for="(c, i) in (item.content as unknown[]) ?? []" :key="i">
        <img
          v-if="partType(c) === 'localImage'"
          class="user-image"
          :src="imageSrc(partPath(c))"
          alt="图片"
        />
        <span v-else-if="partType(c) === 'text'" class="user-text">{{ partText(c) }}</span>
        <span v-else class="mention-inline">@{{ partName(c) }}</span>
      </template>
    </div>
  </div>
  <div v-else-if="item.type === 'agentMessage' || item.type === 'plan'" class="msg msg-agent">
    <span v-if="item.phase === 'commentary' && item.streaming === true" class="phase-badge">进行中</span>
    <MarkdownText :text="String(item.text ?? '')" />
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
      <img class="user-image" :src="imageSrc(String(item.path ?? ''))" alt="图片" />
    </div>
  </div>
  <div v-else-if="item.type === 'sleep'" class="msg">
    <div class="unknown-item">等待 {{ formatDuration(Number(item.durationMs ?? 0)) }}</div>
  </div>
  <div v-else class="msg">
    <div class="unknown-item">{{ item.type }}</div>
  </div>
</template>
