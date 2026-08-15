<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { EditorContent, useEditor } from "@tiptap/vue-3";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { invoke } from "@tauri-apps/api/core";
import type { JSONContent } from "@tiptap/core";
import MentionMenu from "./MentionMenu.vue";
import ModelMenu from "./ModelMenu.vue";
import PermissionMenu from "./PermissionMenu.vue";
import TaskModeMenu from "./TaskModeMenu.vue";
import {
  clearGoal,
  interrupt,
  effectiveEffort,
  modelDisplayName,
  permissionChip,
  registerComposerAddHandler,
  resolveSessionWorkspace,
  sendPrompt,
  setToast,
  store,
  toastError,
  type SessionTab,
  unregisterComposerAddHandler,
} from "../composables/useCodex";
import { useComposerResize } from "../composables/useComposerResize";
import { useContextUsage } from "../composables/useContextUsage";
import { useComposerAttachments } from "../composables/useComposerAttachments";
import type { UserInput } from "../lib/types";
import { assetUrl } from "../lib/asset";
import { debounce } from "../lib/debounce";
import {
  baseName,
  fileMentionSection,
  matchMentionToken,
  MY_REQUEST_MARKER,
  toUserAttachment,
  type FuzzyFileResult,
} from "../lib/mention";
import { permissionMode } from "../lib/permissions";
import { ICON_CHEVRON_DOWN, ICON_GOAL } from "../lib/icons";
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

const props = defineProps<{ tab: SessionTab; active?: boolean }>();

const mention = ref<null | { kind: "@" | "$"; token: string; start: number }>(
  null,
);
const mentionMenu = ref<InstanceType<typeof MentionMenu> | null>(null);

// @ 文件引用：模糊搜索结果
const fileResults = ref<FuzzyFileResult[]>([]);
const searchingFiles = ref(false);
let searchSeq = 0;
const debouncedFileSearch = debounce(
  (token: string) => void runFileSearch(token),
  250,
);

// 编辑器内联引用：chip id → 附件（仅插件/技能）；文件与图片走下方附件区
const refsById = ref(new Map<string, UserInput>());
const rowAttachments = ref<UserInput[]>([]);
const hasText = ref(false);

/** 草稿保存：把当前输入内容快照到会话标签（仅在有内容时写入） */
function saveDraftToTab() {
  if (!props.tab) return;
  const ed = editor.value;
  const json = ed ? (ed.getJSON() as JSONContent) : null;
  const hasDraft =
    hasText.value ||
    rowAttachments.value.length > 0 ||
    refsById.value.size > 0;
  if (!hasDraft) return;
  props.tab.draftJson = json ? JSON.stringify(json) : "";
  props.tab.draftAttachments = [...rowAttachments.value];
  props.tab.draftRefs = Object.fromEntries(refsById.value.entries());
}

/** 草稿恢复：标签激活时还原输入内容与内联引用 */
function restoreDraftFromTab() {
  const ed = editor.value;
  if (!ed || !props.tab?.draftJson) return;
  refsById.value = new Map(Object.entries(props.tab.draftRefs ?? {}));
  rowAttachments.value = [...(props.tab.draftAttachments ?? [])];
  try {
    const json = JSON.parse(props.tab.draftJson) as JSONContent;
    ed.commands.setContent(json);
  } catch {
    // 草稿损坏时静默丢弃，保留空输入
  }
}

// 标签激活状态：激活时恢复草稿，切走前快照
watch(
  () => props.active,
  (v) => {
    if (v) {
      restoreDraftFromTab();
    } else {
      saveDraftToTab();
    }
  },
);

// 输入框可拖拽高度（最低 120px，最高窗口一半）
const {
  composerHeight,
  resizingComposer,
  startComposerResize,
  endComposerResize,
  clampComposerHeightOnResize,
} = useComposerResize();

// 附件摄取：粘贴/拖放 → 附件区（syncAttachments 为函数声明，此处已提升可用）
const {
  dragging,
  handlePasteDom,
  setupDragDrop,
  onDragOver,
  onDragLeave,
  onDrop,
} = useComposerAttachments({
  rowAttachments,
  syncAttachments,
});

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
    handlePaste: (_view, event) => handlePasteDom(event),
  },
  onCreate: () => {
    syncAfterChange();
    exposeEditor();
    registerAddAttachment();
    if (props.active) restoreDraftFromTab();
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

/** 会话资源面板“添加为会话附件”入口：向注册表注册本会话标签的处理器 */
function registerAddAttachment() {
  if (!props.tab) return;
  registerComposerAddHandler(props.tab.id, (a: UserInput) => {
    rowAttachments.value.push(a);
    syncAttachments();
    // 同步进标签草稿：切走/恢复时附件不被草稿恢复覆盖
    props.tab.draftAttachments = [...rowAttachments.value];
    void nextTick(() => editor.value?.commands.focus());
  });
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
      resetFileSearch(true);
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

/** 清空 @ 文件搜索状态；invalidate=true 时使进行中的异步结果失效 */
function resetFileSearch(invalidate = false) {
  fileResults.value = [];
  searchingFiles.value = false;
  debouncedFileSearch.cancel();
  if (invalidate) searchSeq++;
}

function scheduleFileSearch(token: string) {
  if (!token) {
    resetFileSearch(true);
    return;
  }
  debouncedFileSearch.run(token);
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
      setToast(toastError(e));
    }
  } finally {
    if (seq === searchSeq) searchingFiles.value = false;
  }
}

function mentionRoot(): string {
  return resolveSessionWorkspace();
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
    resetFileSearch();
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
  resetFileSearch();
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
  resetFileSearch();
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
      setToast(toastError(e));
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
      setToast(toastError(e));
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

type ComposerMenu = "perm" | "task" | "model";

/** 切换三个按钮菜单：打开一个时关闭另外两个，再点一次当前按钮则关闭 */
function toggleMenu(which: ComposerMenu) {
  const willOpen = !store[`${which}Open`];
  store.permOpen = false;
  store.taskOpen = false;
  store.modelOpen = false;
  store[`${which}Open`] = willOpen;
}

function onKeydownGlobal(e: KeyboardEvent) {
  if (e.key === "Escape") closeMenus();
}

function onWindowMousedown(e: MouseEvent) {
  // 三个按钮弹出层：点击外部任意区域自动关闭。用 mousedown 而非 click，
  // 与 GitView 分支弹层一致——WebView2 原生菜单「粘贴」只合成 click、
  // 不合成 mousedown，避免粘贴等操作误关弹层。
  // 用 Element 而非 HTMLElement：点击 svg/path 等 SVG 目标也应正确判断。
  if (!(e.target instanceof Element)) {
    store.permOpen = false;
    store.taskOpen = false;
    store.modelOpen = false;
    return;
  }
  // 弹出层内部与三个触发按钮不自动关闭（按钮自身的 click 负责切换）
  if (
    e.target.closest(".popup-menu") ||
    e.target.closest(".perm-chip, .task-chip, .model-chip, .goal-chip")
  ) {
    return;
  }
  store.permOpen = false;
  store.taskOpen = false;
  store.modelOpen = false;
}

onMounted(() => {
  window.addEventListener("keydown", onKeydownGlobal);
  window.addEventListener("mousedown", onWindowMousedown);
  window.addEventListener("resize", clampComposerHeightOnResize);
  exposeEditor();
  void setupDragDrop();
  if (props.active) restoreDraftFromTab();
  void nextTick(() => editor.value?.commands.focus());
});
watch(editor, () => {
  exposeEditor();
  registerAddAttachment();
});
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydownGlobal);
  window.removeEventListener("mousedown", onWindowMousedown);
  window.removeEventListener("resize", clampComposerHeightOnResize);
  if (props.tab) unregisterComposerAddHandler(props.tab.id);
  endComposerResize();
  debouncedFileSearch.cancel();
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
  // 目标 flag：仅执行模式（非计划）下首条消息消费勾选，目标=该消息纯文本；
  // 计划模式消息不消费，arm 保持（“执行计划”按钮另行以计划内容挂载目标）
  if (store.goalArmed && store.taskMode !== "plan" && plainText.trim()) {
    store.goalText = plainText.trim();
    store.goalArmed = false;
    store.goalStatus = null;
  }
  const rowItems = rowAttachments.value;
  const files = rowItems.filter((a) => a.type === "mention");
  const fileSection = fileMentionSection(files);
  const marker = files.length ? `\n${MY_REQUEST_MARKER}\n` : "";
  const wireText = `${fileSection}${marker}${wireInline}`;
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

function rowAttPath(a: UserInput): string {
  return a.type === "mention" || a.type === "localImage" ? a.path : "";
}

function rowAttName(a: UserInput): string {
  return a.type === "mention" || a.type === "skill" ? a.name : "";
}

// 上下文窗口使用情况与手动压缩
const { ctxUsage, ctxTooltip, compacting, compactNow } = useContextUsage();

function modelChipLabel(): string {
  const name = modelDisplayName(store.model);
  const effort = effectiveEffort();
  return effort ? `${name} (${effort})` : name;
}

function taskModeLabel(): string {
  if (store.taskMode === "plan") return "计划模式";
  return "执行模式";
}

/** 目标预览：压缩连续空白并截断到 120 字符，超长追加省略号 */
function goalPreview(text: string): string {
  const t = text.replace(/\s+/g, " ").trim();
  return t.length > 120 ? `${t.slice(0, 120)}…` : t;
}

/** 目标旗子提示：未勾选/已勾选待首条消息/已挂载（进行中） */
function goalTooltip(): string {
  if (!store.goalText && !store.goalArmed) {
    return "目标：勾选后，首条消息将作为目标自动执行";
  }
  if (store.goalArmed && !store.goalText) {
    return "目标已勾选：发送首条消息后自动以其为目标，点击取消勾选";
  }
  const text = store.goalText ?? "";
  if (!store.currentThreadId) return `目标：${goalPreview(text)}（待挂载）`;
  return `目标：${goalPreview(text)}（进行中），点击取消目标`;
}

/** 目标旗子：已挂载/回填目标时点击直接取消（clearGoal）；否则切换勾选态 */
function onGoalIconClick() {
  if (store.goalText) {
    void clearGoal();
    return;
  }
  store.goalArmed = !store.goalArmed;
}

</script>

<template>
  <div
    class="composer"
    :class="{ dragover: dragging }"
    @dragover.prevent="onDragOver()"
    @dragleave="onDragLeave($event)"
    @drop="onDrop($event)"
  >
    <div
      class="composer-input-row"
      :style="composerHeight ? { '--composer-h': `${composerHeight}px` } : undefined"
    >
      <div
        class="composer-resize-handle"
        :class="{ active: resizingComposer }"
        aria-label="调整输入框高度"
        v-tooltip="'拖动调整输入框高度'"
        @pointerdown="startComposerResize"
      >
        <span class="composer-resize-grip"></span>
      </div>
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
            @click="toggleMenu('perm')"
          >
            <svg class="chip-icon" viewBox="0 0 24 24">
              <path :d="permissionMode(store.permissionMode).icon" />
            </svg>
            {{ permissionChip() }}
            <svg class="chevron" viewBox="0 0 16 16">
              <path :d="ICON_CHEVRON_DOWN" />
            </svg>
          </button>
          <PermissionMenu v-if="store.permOpen" @close="store.permOpen = false" />
        </div>
        <div class="menu-anchor">
          <button
            class="task-chip"
            v-tooltip="'任务模式'"
            :disabled="store.turnActive"
            @click="toggleMenu('task')"
          >
            <svg class="chip-icon" viewBox="0 0 24 24">
              <path :d="taskMode(store.taskMode).icon" />
            </svg>
            {{ taskModeLabel() }}
            <svg viewBox="0 0 16 16">
            <path :d="ICON_CHEVRON_DOWN" />
            </svg>
          </button>
          <TaskModeMenu v-if="store.taskOpen" @close="store.taskOpen = false" />
        </div>
        <div class="menu-anchor">
          <div
            v-if="!store.turnActive || !!store.goalText"
            class="goal-chip"
            :class="{ 'has-goal': !!store.goalText || store.goalArmed }"
          >
            <button
              class="goal-icon-btn"
              :class="{
                'has-goal': !!store.goalText || store.goalArmed,
                'status-active':
                  !!store.goalText &&
                  (store.goalStatus === 'active' || store.goalStatus === null),
              }"
              :aria-pressed="!!store.goalText || store.goalArmed"
              :aria-label="goalTooltip()"
              v-tooltip="goalTooltip()"
              @click="onGoalIconClick()"
            >
              <svg viewBox="0 0 24 24">
                <path :d="ICON_GOAL" />
              </svg>
              <span
                v-if="store.goalArmed || store.goalText"
                class="goal-check"
                aria-hidden="true"
              ></span>
            </button>
          </div>
        </div>
      </div>
      <div class="composer-right">
        <button
          v-if="ctxUsage"
          class="ctx-window"
          aria-label="压缩上下文"
          :disabled="!store.currentThreadId || compacting"
          v-tooltip="ctxTooltip"
          @dblclick="compactNow()"
        >
          {{ ctxUsage.pct }}%
        </button>
        <div class="menu-anchor">
          <button
            class="model-chip"
            v-tooltip="'模型'"
            @click="toggleMenu('model')"
          >
            <svg class="model-chip-icon" viewBox="0 0 24 24">
              <rect x="5" y="5" width="14" height="14" rx="2" />
              <rect x="9.5" y="9.5" width="5" height="5" rx="1" />
              <path d="M9 2v3M15 2v3M9 19v3M15 19v3M2 9h3M2 15h3M19 9h3M19 15h3" />
            </svg>
            {{ modelChipLabel() }}
            <svg viewBox="0 0 16 16">
              <path :d="ICON_CHEVRON_DOWN" />
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
          :src="assetUrl(a.path)"
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

    <Teleport to="body">
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
    </Teleport>
  </div>
</template>
