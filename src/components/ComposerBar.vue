<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { EditorContent, useEditor } from "@tiptap/vue-3";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import MentionMenu from "./MentionMenu.vue";
import ModelMenu from "./ModelMenu.vue";
import PermissionMenu from "./PermissionMenu.vue";
import TaskModeMenu from "./TaskModeMenu.vue";
import {
  interrupt,
  effectiveEffort,
  modelDisplayName,
  permissionChip,
  sendPrompt,
  store,
  toastError,
} from "../composables/useCodex";
import type { UserInput } from "../lib/types";
import {
  baseName,
  fileMentionSection,
  matchMentionToken,
  MY_REQUEST_MARKER,
  toUserAttachment,
  type FuzzyFileResult,
} from "../lib/mention";
import { permissionMode } from "../lib/permissions";
import {
  Reference,
  docToRuns,
  refKindOfAttachment,
  runsToText,
  runsToWireText,
  tokenStartPos,
  type EditorRun,
} from "../lib/richEditor";
import { taskMode } from "../lib/tasks";

const mention = ref<null | { kind: "@" | "$"; token: string; start: number }>(
  null,
);
const mentionMenu = ref<InstanceType<typeof MentionMenu> | null>(null);

// @ 文件引用：模糊搜索结果
const fileResults = ref<FuzzyFileResult[]>([]);
const searchingFiles = ref(false);
let searchSeq = 0;
let searchTimer: number | undefined;

// 编辑器内联引用：chip id → 附件（仅插件/技能）；文件与图片走下方附件区
const refsById = ref(new Map<string, UserInput>());
const rowAttachments = ref<UserInput[]>([]);
const hasText = ref(false);

// 用户输入历史（仅内存），供向上/向下键选择，行为类似 Linux shell
const sentHistory: string[] = [];
let historyIndex = -1;

const editor = useEditor({
  extensions: [
    StarterKit,
    Placeholder.configure({
      placeholder: "输入消息，@ 引用文件或插件 / $ 调用技能…",
    }),
    Reference,
  ],
  editorProps: {
    attributes: {
      class: "rich-input",
      role: "textbox",
      "aria-multiline": "true",
    },
    handleKeyDown: (_view, event) => handleKeydown(event),
  },
  onCreate: () => {
    syncAfterChange();
    exposeEditor();
  },
  onUpdate: () => {
    syncAfterChange();
  },
  onSelectionUpdate: () => {
    updateMentionFromCaret();
  },
});

function exposeEditor() {
  try {
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ =
      editor.value;
  } catch {
    // 非浏览器环境忽略
  }
}

function currentRuns(): EditorRun[] {
  const ed = editor.value;
  if (!ed) return [];
  return docToRuns(ed.getJSON());
}

function plainTextBeforeCaret(): string {
  const ed = editor.value;
  if (!ed) return "";
  const pos = ed.state.selection.from;
  return ed.state.doc.textBetween(0, pos, "\n", () => "");
}

function updateMentionFromCaret() {
  const textBefore = plainTextBeforeCaret();
  const m = matchMentionToken(textBefore);
  mention.value = m;
  if (!m) {
    fileResults.value = [];
    searchingFiles.value = false;
    if (searchTimer) window.clearTimeout(searchTimer);
    searchSeq++;
    return;
  }
  if (m.kind === "@") scheduleFileSearch(m.token);
}

function syncAfterChange() {
  updateMentionFromCaret();
  const runs = currentRuns();
  hasText.value = runsToText(runs).trim().length > 0;
  syncAttachments();
}

function syncAttachments() {
  const runs = currentRuns();
  const refs = runs
    .filter((r) => r.kind === "ref")
    .map((r) => (r.kind === "ref" ? refsById.value.get(r.refId) : undefined))
    .filter((a): a is UserInput => !!a);
  store.attachments = [...refs, ...rowAttachments.value];
}

function scheduleFileSearch(token: string) {
  if (searchTimer) window.clearTimeout(searchTimer);
  if (!token) {
    fileResults.value = [];
    searchingFiles.value = false;
    searchSeq++;
    return;
  }
  searchTimer = window.setTimeout(() => {
    void runFileSearch(token);
  }, 250);
}

async function runFileSearch(token: string) {
  const seq = ++searchSeq;
  const root = mentionRoot();
  if (!root) {
    searchingFiles.value = false;
    return;
  }
  searchingFiles.value = true;
  try {
    const res = await invoke<{ files?: FuzzyFileResult[] }>("codex_rpc", {
      method: "fuzzyFileSearch",
      params: { query: token, roots: [root], cancellationToken: null },
    });
    if (seq !== searchSeq) return;
    fileResults.value = (res.files ?? []).slice(0, 50);
  } catch (e) {
    if (seq === searchSeq) {
      fileResults.value = [];
      store.toast = toastError(e);
    }
  } finally {
    if (seq === searchSeq) searchingFiles.value = false;
  }
}

function mentionRoot(): string {
  return (
    store.newChatCwd ?? store.currentThreadCwd ?? store.server.workspace ?? ""
  );
}

function refNameOf(a: UserInput): string {
  if (a.type === "mention" || a.type === "skill") return a.name;
  return "";
}

/** 菜单选中引用：插件/技能删除触发词后插入内联 chip；文件/图片删除触发词后进附件区 */
function onSelectAttachment(a: UserInput) {
  const ed = editor.value;
  if (!ed) return;
  const m = mention.value;
  const caretPos = ed.state.selection.from;
  const from = m
    ? tokenStartPos(ed.state.doc, caretPos, m.token.length)
    : caretPos;
  if (a.type === "mention" || a.type === "localImage") {
    ed.chain().focus().deleteRange({ from, to: caretPos }).run();
    mention.value = null;
    if (searchTimer) window.clearTimeout(searchTimer);
    fileResults.value = [];
    searchingFiles.value = false;
    rowAttachments.value.push(a);
    syncAttachments();
    void nextTick(() => ed.commands.focus());
    return;
  }
  const id = `ref-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  refsById.value.set(id, a);
  ed.chain()
    .focus()
    .deleteRange({ from, to: caretPos })
    .insertContent([
      {
        type: "reference",
        attrs: { refId: id, kind: refKindOfAttachment(a), label: refNameOf(a) },
      },
      { type: "text", text: " " },
    ])
    .run();
  mention.value = null;
  if (searchTimer) window.clearTimeout(searchTimer);
  fileResults.value = [];
  searchingFiles.value = false;
  syncAttachments();
  void nextTick(() => ed.commands.focus());
}

/** 删除当前触发词（用于打开本地文件/文件夹选择器前） */
function removeMentionTokenInEditor() {
  const ed = editor.value;
  const m = mention.value;
  if (!ed || !m) return;
  const caretPos = ed.state.selection.from;
  const from = tokenStartPos(ed.state.doc, caretPos, m.token.length);
  ed.chain().focus().deleteRange({ from, to: caretPos }).run();
  mention.value = null;
  if (searchTimer) window.clearTimeout(searchTimer);
  fileResults.value = [];
  searchingFiles.value = false;
}

function onPickFiles() {
  removeMentionTokenInEditor();
  void (async () => {
    try {
      const files = await invoke<string[]>("pick_files", {
        multiple: true,
        initialDir: mentionRoot(),
      });
      for (const f of files) {
        const a = toUserAttachment(baseName(f), f);
        rowAttachments.value.push(a);
      }
      syncAttachments();
    } catch (e) {
      store.toast = toastError(e);
    } finally {
      void nextTick(() => editor.value?.commands.focus());
    }
  })();
}

function onPickDir() {
  removeMentionTokenInEditor();
  void (async () => {
    try {
      const dir = await invoke<string | null>("pick_directory", {
        initialDir: mentionRoot(),
      });
      if (dir) {
        const a = toUserAttachment(baseName(dir), dir);
        rowAttachments.value.push(a);
      }
      syncAttachments();
    } catch (e) {
      store.toast = toastError(e);
    } finally {
      void nextTick(() => editor.value?.commands.focus());
    }
  })();
}

function closeMenus() {
  store.permOpen = false;
  store.taskOpen = false;
  store.modelOpen = false;
  mention.value = null;
}

function onKeydownGlobal(e: KeyboardEvent) {
  if (e.key === "Escape") closeMenus();
}

onMounted(() => {
  window.addEventListener("keydown", onKeydownGlobal);
  exposeEditor();
  void nextTick(() => editor.value?.commands.focus());
});
watch(editor, exposeEditor);
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydownGlobal);
  try {
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ =
      undefined;
  } catch {
    // ignore
  }
});

// 新建对话后输入框重新获得焦点（组件未卸载的情况，如聊天页直接点“新建对话”）
watch(
  () => store.currentThreadId,
  (v) => {
    if (!v) void nextTick(() => editor.value?.commands.focus());
  },
);

function handleKeydown(e: KeyboardEvent): boolean {
  // 输入法组合中（如中文拼音选字）的按键不触发提交/历史选择
  if (e.isComposing || e.keyCode === 229) return false;
  // @ / $ 菜单打开时：Enter 选中高亮项，↑↓ 移动高亮
  if (
    mention.value &&
    (e.key === "ArrowUp" ||
      e.key === "ArrowDown" ||
      (e.key === "Enter" && !e.ctrlKey))
  ) {
    e.preventDefault();
    if (e.key === "Enter") {
      mentionMenu.value?.selectHighlighted();
    } else {
      mentionMenu.value?.move(e.key === "ArrowUp" ? -1 : 1);
    }
    return true;
  }
  if (e.key === "Enter") {
    // Enter 快捷发送：开启时 Enter 发送 / Ctrl+Enter 换行 / Shift+Enter 无操作；
    // 关闭时 Enter 换行 / Ctrl+Enter 发送
    if (store.settings.enter_to_send) {
      if (e.ctrlKey && !e.shiftKey) {
        e.preventDefault();
        editor.value?.commands.setHardBreak();
        return true;
      }
      if (e.shiftKey) {
        e.preventDefault();
        return true;
      }
      e.preventDefault();
      submit(false);
      return true;
    }
    if (e.ctrlKey) {
      e.preventDefault();
      submit(true);
      return true;
    }
    return false;
  }
  if (e.key === "ArrowUp" || e.key === "ArrowDown") {
    // 仅当光标位于文档开头时触发历史选择，否则保留默认的光标移动
    const sel = editor.value?.state.selection;
    if (!sel || !sel.empty || sel.from > 1) return false;
    e.preventDefault();
    if (e.key === "ArrowUp") {
      if (!sentHistory.length) return true;
      if (historyIndex === -1) historyIndex = sentHistory.length - 1;
      else if (historyIndex > 0) historyIndex--;
      setEditorPlainText(sentHistory[historyIndex]);
    } else {
      if (historyIndex === -1) return true;
      historyIndex++;
      if (historyIndex >= sentHistory.length) {
        historyIndex = -1;
        setEditorPlainText("");
      } else {
        setEditorPlainText(sentHistory[historyIndex]);
      }
    }
    return true;
  }
  return false;
}

function setEditorPlainText(text: string) {
  editor.value?.commands.setContent(text);
  const ed = editor.value;
  if (ed) {
    const end = ed.state.doc.content.size;
    ed.chain().focus().setTextSelection(end).run();
  }
}

function submit(flip = false) {
  closeMenus(); // 发送后关闭可能开着的菜单，避免回合中还能切换模式
  const runs = currentRuns();
  const wireInline = runsToWireText(runs, refsById.value);
  const plainText = runsToText(runs);
  const refs = runs
    .filter((r) => r.kind === "ref")
    .map((r) => (r.kind === "ref" ? refsById.value.get(r.refId) : undefined))
    .filter((a): a is UserInput => !!a);
  const rowItems = rowAttachments.value;
  const files = rowItems.filter((a) => a.type === "mention");
  const fileSection = fileMentionSection(files);
  const marker = files.length ? `\n${MY_REQUEST_MARKER}\n` : "";
  const wireText = `${fileSection}${marker}${wireInline}`;
  // 目标模式：首条消息的纯文本即目标（后续回合完成/终止时清除）
  if (store.taskMode === "goal" && plainText.trim()) {
    store.goalText = plainText.trim();
  }
  // 先清空编辑器（onUpdate 会同步 store.attachments 为空），再写入本次附件
  editor.value?.commands.setContent("");
  refsById.value = new Map();
  rowAttachments.value = [];
  store.attachments = [...refs, ...rowItems];
  if (plainText.trim()) {
    sentHistory.push(plainText);
    if (sentHistory.length > 100) sentHistory.shift();
  }
  historyIndex = -1;
  void sendPrompt(wireText, flip);
}

function removeRowAttachment(i: number) {
  rowAttachments.value.splice(i, 1);
  syncAttachments();
}

function imageSrc(path: string): string {
  try {
    return convertFileSrc(path);
  } catch {
    return path;
  }
}

function rowAttPath(a: UserInput): string {
  return a.type === "mention" || a.type === "localImage" ? a.path : "";
}

function rowAttName(a: UserInput): string {
  return a.type === "mention" || a.type === "skill" ? a.name : "";
}

const newChatCwdLabel = computed(() => store.newChatCwd ?? store.server.workspace);

// 上下文窗口使用情况：window 未知时不显示
const ctxUsage = computed(() => {
  const u = store.threadTokenUsage;
  if (!u || u.window == null || u.window <= 0) return null;
  return {
    pct: Math.min(100, Math.round((u.used / u.window) * 100)),
    used: u.used,
    window: u.window,
  };
});

const ctxTooltip = computed(() => {
  const u = ctxUsage.value;
  if (!u) return "";
  return `上下文已用 ${formatTokens(u.used)}，共 ${formatTokens(u.window)}`;
});

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return String(n);
}

async function pickNewChatCwd() {
  try {
    const dir = await invoke<string | null>("pick_directory");
    if (dir) store.newChatCwd = dir;
  } catch (e) {
    store.toast = toastError(e);
  }
}

function modelChipLabel(): string {
  const name = modelDisplayName(store.model);
  const effort = effectiveEffort();
  return effort ? `${name}(${effort})` : name;
}

function taskModeLabel(): string {
  if (store.taskMode === "plan") return "计划模式";
  if (store.taskMode === "goal") return "目标模式";
  return "执行模式";
}

</script>

<template>
  <div class="composer">
    <div v-if="!store.currentThreadId" class="newchat-cwd-row">
      <svg viewBox="0 0 24 24">
        <path
          d="M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"
        />
      </svg>
      <span class="newchat-cwd-label">项目目录</span>
      <button
        class="newchat-cwd-value"
        v-tooltip="newChatCwdLabel"
        @click="pickNewChatCwd()"
      >
        {{ newChatCwdLabel }}
      </button>
      <button
        v-if="store.newChatCwd"
        class="newchat-cwd-reset"
        aria-label="恢复默认工作目录"
        v-tooltip="'恢复默认工作目录'"
        @click="store.newChatCwd = null"
      >
        ×
      </button>
    </div>
    <div class="composer-input-row">
      <div class="menu-anchor input-anchor">
        <EditorContent :editor="editor" class="rich-editor" />
        <MentionMenu
          ref="mentionMenu"
          v-if="mention"
          :kind="mention.kind"
          :token="mention.token"
          :results="fileResults"
          :searching="searchingFiles"
          @close="mention = null"
          @pick-files="onPickFiles()"
          @pick-dir="onPickDir()"
          @select-attachment="onSelectAttachment($event)"
        />
      </div>
      <div class="composer-left">
        <div class="menu-anchor">
          <button
            class="perm-chip"
            v-tooltip="'权限模式'"
            :disabled="store.turnActive"
            @click="store.permOpen = !store.permOpen"
          >
            <svg class="chip-icon" viewBox="0 0 24 24">
              <path :d="permissionMode(store.permissionMode).icon" />
            </svg>
            {{ permissionChip() }}
            <svg class="chevron" viewBox="0 0 16 16">
              <path d="M4 6l4 4 4-4z" />
            </svg>
          </button>
          <PermissionMenu v-if="store.permOpen" @close="store.permOpen = false" />
        </div>
        <div class="menu-anchor">
          <button
            class="task-chip"
            v-tooltip="'任务模式'"
            :disabled="store.turnActive"
            @click="store.taskOpen = !store.taskOpen"
          >
            <svg class="chip-icon" viewBox="0 0 24 24">
              <path :d="taskMode(store.taskMode).icon" />
            </svg>
            {{ taskModeLabel() }}
            <svg viewBox="0 0 16 16">
              <path d="M4 6l4 4 4-4z" />
            </svg>
          </button>
          <TaskModeMenu v-if="store.taskOpen" @close="store.taskOpen = false" />
        </div>
      </div>
      <div class="composer-right">
        <span v-if="ctxUsage" class="ctx-window" v-tooltip="ctxTooltip">
          {{ ctxUsage.pct }}%
        </span>
        <div class="menu-anchor">
          <button
            class="model-chip"
            v-tooltip="'模型'"
            @click="store.modelOpen = !store.modelOpen"
          >
            <svg class="model-chip-icon" viewBox="0 0 24 24">
              <rect x="5" y="5" width="14" height="14" rx="2" />
              <rect x="9.5" y="9.5" width="5" height="5" rx="1" />
              <path d="M9 2v3M15 2v3M9 19v3M15 19v3M2 9h3M2 15h3M19 9h3M19 15h3" />
            </svg>
            {{ modelChipLabel() }}
            <svg viewBox="0 0 16 16">
              <path d="M4 6l4 4 4-4z" />
            </svg>
          </button>
          <ModelMenu v-if="store.modelOpen" @close="store.modelOpen = false" />
        </div>
        <button
          v-if="store.turnActive"
          class="send-btn stop"
          v-tooltip="'停止生成'"
          @click="interrupt()"
        >
          <svg viewBox="0 0 24 24">
            <rect x="6" y="6" width="12" height="12" rx="1.5" />
          </svg>
          停止
        </button>
        <button
          v-else
          class="send-btn"
          v-tooltip="'发送'"
          :class="{ lit: !!(hasText || store.attachments.length) }"
          :disabled="!hasText && store.attachments.length === 0"
          @click="submit()"
        >
          <svg viewBox="0 0 24 24">
            <path d="M12 19V5M5 12l7-7 7 7" />
          </svg>
          发送
        </button>
      </div>
    </div>
    <div v-if="rowAttachments.length" class="attachment-row">
      <span
        v-for="(a, i) in rowAttachments"
        :key="i"
        class="attachment-chip"
        v-tooltip="rowAttPath(a)"
      >
        <img
          v-if="a.type === 'localImage'"
          class="attachment-thumb"
          :src="imageSrc(a.path)"
          alt=""
        />
        <template v-if="a.type === 'localImage'">
          {{ a.path.split(/[\\/]/).pop() ?? a.path }}
        </template>
        <template v-else>@{{ rowAttName(a) }}</template>
        <button
          aria-label="移除附件"
          v-tooltip="'移除附件'"
          @click="removeRowAttachment(i)"
        >
          ×
        </button>
      </span>
    </div>

    <div
      v-if="
        store.permOpen ||
        store.taskOpen ||
        store.modelOpen ||
        mention
      "
      class="menu-backdrop"
      @click="closeMenus()"
    ></div>
  </div>
</template>
