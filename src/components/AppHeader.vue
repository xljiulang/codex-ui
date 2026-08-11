<script setup lang="ts">
import { computed, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { newEmptyChat, store, toastError } from "../composables/useCodex";

// 新对话态显示已选择目录；会话态显示线程固定目录；兜底启动工作目录
const cwd = computed(() =>
  store.currentThreadId
    ? store.currentThreadCwd ?? store.server.workspace
    : store.newChatCwd ?? store.server.workspace,
);
// 会话未开始（新对话态）目录可修改；会话进行中线程目录已固定，只读
const isNewChat = computed(() => !store.currentThreadId);

// 只读展示；点击在资源管理器中打开
function openInExplorer() {
  if (!cwd.value) return;
  void invoke("open_url", { url: cwd.value }).catch(() => undefined);
}

async function pickNewChatCwd() {
  try {
    const dir = await invoke<string | null>("pick_directory");
    if (dir) store.newChatCwd = dir;
  } catch (e) {
    store.toast = toastError(e);
  }
}

function onCwdClick() {
  if (isNewChat.value) void pickNewChatCwd();
  else openInExplorer();
}

function onNewChat() {
  store.showSettings = false;
  store.showHistory = false;
  void newEmptyChat();
  // 无论是否发生了会话切换，新建对话后都让输入框重新获得焦点
  void nextTick(focusComposer);
}

/** 聚焦输入框：优先用 ComposerBar 暴露的 Tiptap 实例（可正确放置光标），兜底聚焦 ProseMirror DOM */
function focusComposer() {
  const ed = (
    window as unknown as {
      __CODEX_UI_EDITOR__?: { commands?: { focus?: () => void } };
    }
  ).__CODEX_UI_EDITOR__;
  if (ed?.commands?.focus) {
    ed.commands.focus();
    return;
  }
  document.querySelector<HTMLElement>(".composer .ProseMirror")?.focus();
}

function onSettings() {
  store.showHistory = false;
  store.showSettings = !store.showSettings;
}

function onHistory() {
  store.showSettings = false;
  store.showHistory = !store.showHistory;
}
</script>

<template>
  <header class="app-header">
    <div class="brand">
      <span class="brand-logo">
        <svg viewBox="0 0 24 24">
          <path d="M12 2l8.66 5v10L12 22l-8.66-5V7z" />
          <path class="logo-c" d="M14.9 9.1a4.5 4.5 0 1 0 0 5.8" />
        </svg>
      </span>
      <span class="brand-name">CODEX</span>
      <button
        v-if="cwd"
        class="brand-cwd"
        :class="{ pickable: isNewChat }"
        v-tooltip="isNewChat ? '点击可修改工作目录' : `在资源管理器中打开：${cwd}`"
        @click="onCwdClick()"
      >
        <svg viewBox="0 0 24 24">
          <path
            d="M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"
          />
        </svg>
        {{ cwd }}
      </button>
      <button
        v-if="isNewChat && store.newChatCwd"
        class="brand-cwd-reset"
        aria-label="恢复默认工作目录"
        v-tooltip="'恢复默认工作目录'"
        @click="store.newChatCwd = null"
      >
        ×
      </button>
    </div>
    <div class="header-actions">
      <button
        class="icon-btn"
        aria-label="历史记录"
        v-tooltip="'历史记录'"
        :class="{ active: store.showHistory }"
        @click="onHistory()"
      >
        <svg viewBox="0 0 24 24">
          <path
            d="M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm0 18a8 8 0 1 1 8-8 8 8 0 0 1-8 8zm1-13h-2v6l5.25 3.15 1-1.65L13 12z"
          />
        </svg>
      </button>
      <button
        class="icon-btn"
        aria-label="设置"
        v-tooltip="'设置'"
        :class="{ active: store.showSettings }"
        @click="onSettings()"
      >
        <svg viewBox="0 0 24 24">
          <path
            d="M19.14 12.94a7.1 7.1 0 0 0 .06-.94 7.1 7.1 0 0 0-.06-.94l2.03-1.58a.5.5 0 0 0 .12-.64l-1.92-3.32a.5.5 0 0 0-.61-.22l-2.39.96a7.1 7.1 0 0 0-1.62-.94L14.4 2.8a.5.5 0 0 0-.49-.4h-3.82a.5.5 0 0 0-.49.4l-.36 2.54a7.1 7.1 0 0 0-1.62.94l-2.39-.96a.5.5 0 0 0-.61.22l-1.92 3.32a.5.5 0 0 0 .12.64l2.03 1.58a7.1 7.1 0 0 0 0 1.88l-2.03 1.58a.5.5 0 0 0-.12.64l1.92 3.32a.5.5 0 0 0 .61.22l2.39-.96a7.1 7.1 0 0 0 1.62.94l.36 2.54a.5.5 0 0 0 .49.4h3.82a.5.5 0 0 0 .49-.4l.36-2.54a7.1 7.1 0 0 0 1.62-.94l2.39.96a.5.5 0 0 0 .61-.22l1.92-3.32a.5.5 0 0 0-.12-.64zM12 15.5A3.5 3.5 0 1 1 15.5 12 3.5 3.5 0 0 1 12 15.5z"
          />
        </svg>
      </button>
      <button
        class="icon-btn"
        aria-label="新建对话"
        v-tooltip="'新建对话'"
        @click="onNewChat()"
      >
        <svg viewBox="0 0 24 24">
          <path d="M11 5h2v6h6v2h-6v6h-2v-6H5v-2h6z" />
        </svg>
      </button>
    </div>
  </header>
</template>
