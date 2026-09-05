<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { EditorContent, useEditor } from "@tiptap/vue-3";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { invoke } from "@tauri-apps/api/core";
import MentionMenu from "./MentionMenu.vue";
import ModelMenu from "./ModelMenu.vue";
import PermissionMenu from "./PermissionMenu.vue";
import CollaborationModeMenu from "./CollaborationModeMenu.vue";
import ContextUsageMenu from "./ContextUsageMenu.vue";
import {
  interrupt,
  effectiveEffort,
  modelDisplayName,
  permissionChip,
  registerComposerAddHandler,
  sendPrompt,
  setToast,
  store,
  toastError,
  workspace,
  type SessionTab,
  unregisterComposerAddHandler,
} from "../composables/useCodex";
import { useComposerResize } from "../composables/useComposerResize";
import { useComposerAttachments } from "../composables/useComposerAttachments";
import { useComposerDraft } from "../composables/useComposerDraft";
import { useComposerMenus } from "../composables/useComposerMenus";
import { useMentionFileSearch } from "../composables/useMentionFileSearch";
import { useInputHistory } from "../composables/useInputHistory";
import type { UserInput } from "../lib/types";
import { assetUrl } from "../lib/asset";
import {
  baseName,
  fileMentionSection,
  matchMentionToken,
  MY_REQUEST_MARKER,
  toUserAttachment,
} from "../lib/mention";
import { permissionMode } from "../lib/permissions";
import { ICON_CHEVRON_DOWN, ICON_MODEL_CUBE } from "../lib/icons";
import GoalChip from "./GoalChip.vue";
import {
  Reference,
  docToRuns,
  refKindOfAttachment,
  runsToText,
  runsToWireText,
  tokenStartPos,
  type EditorRun,
} from "../lib/richEditor";
import { collaborationMode } from "../lib/collaborationModes";

const props = defineProps<{ tab: SessionTab; active?: boolean }>();

const mention = ref<null | { kind: "@" | "$"; token: string; start: number }>(
  null,
);
// 输入区三个按钮菜单（权限/协作模式/模型）开关与外部点击关闭；
// 状态为组件局部 ref，多会话标签各自的输入区互不串扰
const {
  permOpen,
  collabOpen,
  modelOpen,
  closeMenus,
  toggleMenu,
  onKeydownGlobal,
  onWindowMousedown,
} = useComposerMenus({ mention });
const mentionMenu = ref<InstanceType<typeof MentionMenu> | null>(null);

// @ 文件引用：模糊搜索（防抖/序号失效/上限 50）
const {
  fileResults,
  searchingFiles,
  scheduleFileSearch,
  resetFileSearch,
  cancelFileSearch,
} = useMentionFileSearch();

// 编辑器内联引用：chip id → 附件（仅插件/技能）；文件与图片走下方附件区
const refsById = ref(new Map<string, UserInput>());
const rowAttachments = ref<UserInput[]>([]);
const hasText = ref(false);

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
const inputHistory = useInputHistory();

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

// 输入草稿：按标签持久化（激活恢复、切走快照）
const { saveDraftToTab, restoreDraftFromTab } = useComposerDraft({
  tab: () => props.tab,
  editor,
  rowAttachments,
  refsById,
  hasText,
});

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
  props.tab.attachments = [...refs, ...rowAttachments.value];
}

function refNameOf(a: UserInput): string {
  if (a.type === "mention" || a.type === "skill") return a.name;
  return "";
}

/** 文件/文件夹选择器的初始目录（与 @ 搜索同一工作区基准） */
function mentionRoot(): string {
  return workspace.value;
}

/** 菜单选中引用：插件/技能删除触发词后插入内联 chip；文件/图片删除触发词后进附件区 */
function onSelectFile(a: UserInput) {
  const ed = editor.value;
  if (!ed) return;
  const m = mention.value;
  const caretPos = ed.state.selection.from;
  const from = m
    ? tokenStartPos(ed.state.doc, caretPos, m.token.length)
    : caretPos;
  if (a.type === "mention" || a.type === "localImage") {
    ed.chain().focus().deleteRange({ from, to: caretPos }).run();
    closeMentionMenu();
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
  closeMentionMenu();
  syncAttachments();
  void nextTick(() => ed.commands.focus());
}

/** 关闭 @ 菜单并清空搜索状态（选中/取消/删除触发词共用） */
function closeMentionMenu() {
  mention.value = null;
  resetFileSearch();
}

/** 删除当前触发词（用于打开本地文件/文件夹选择器前） */
function removeMentionTokenInEditor() {
  const ed = editor.value;
  const m = mention.value;
  if (!ed || !m) return;
  const caretPos = ed.state.selection.from;
  const from = tokenStartPos(ed.state.doc, caretPos, m.token.length);
  ed.chain().focus().deleteRange({ from, to: caretPos }).run();
  closeMentionMenu();
}

/** 文件/文件夹选择器共用流程：删除触发词 → 选择 → 附件区 → 回焦 */
async function pickAndAdd(picker: () => Promise<string[] | string | null>) {
  removeMentionTokenInEditor();
  try {
    const picked = await picker();
    const list = Array.isArray(picked) ? picked : picked ? [picked] : [];
    for (const p of list) {
      const a = toUserAttachment(baseName(p), p);
      rowAttachments.value.push(a);
    }
    syncAttachments();
  } catch (e) {
    setToast(toastError(e));
  } finally {
    void nextTick(() => editor.value?.commands.focus());
  }
}

function onPickFiles() {
  void pickAndAdd(() =>
    invoke<string[]>("pick_files", {
      multiple: true,
      initialDir: mentionRoot(),
    }),
  );
}

function onPickDir() {
  void pickAndAdd(() =>
    invoke<string | null>("pick_directory", { initialDir: mentionRoot() }),
  );
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
  cancelFileSearch();
  try {
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ =
      undefined;
  } catch {
    // ignore
  }
});

// 新建对话后输入框重新获得焦点（组件未卸载的情况，如聊天页直接点“新建对话”）
watch(
  () => props.tab.threadId,
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
      const t = inputHistory.prev();
      if (t !== null) setEditorPlainText(t);
    } else {
      const t = inputHistory.next();
      if (t !== null) setEditorPlainText(t);
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

function submit(invertFollowup = false) {
  closeMenus(); // 发送后关闭可能开着的菜单，避免回合中还能切换模式
  const runs = currentRuns();
  const wireInline = runsToWireText(runs, refsById.value);
  const plainText = runsToText(runs);
  const refs = runs
    .filter((r) => r.kind === "ref")
    .map((r) => (r.kind === "ref" ? refsById.value.get(r.refId) : undefined))
    .filter((a): a is UserInput => !!a);
  // 目标 flag：仅默认模式（非计划）下首条消息消费勾选，目标=该消息纯文本；
  // 计划模式消息不消费，arm 保持（“执行计划”按钮另行以计划内容挂载目标）
  if (props.tab.goalArmed && props.tab.collaborationMode !== "plan" && plainText.trim()) {
    props.tab.goalText = plainText.trim();
    props.tab.goalArmed = false;
    props.tab.goalStatus = null;
  }
  const rowItems = rowAttachments.value;
  const files = rowItems.filter((a) => a.type === "mention");
  const fileSection = fileMentionSection(files);
  const marker = files.length ? `\n${MY_REQUEST_MARKER}\n` : "";
  const wireText = `${fileSection}${marker}${wireInline}`;
  // 先清空编辑器（onUpdate 会同步 tab.attachments 为空），再写入本次附件
  editor.value?.commands.setContent("");
  refsById.value = new Map();
  rowAttachments.value = [];
  props.tab.attachments = [...refs, ...rowItems];
  if (plainText.trim()) {
    inputHistory.push(plainText);
  }
  void sendPrompt(wireText, invertFollowup);
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

function modelChipLabel(): string {
  const name = modelDisplayName(props.tab.model ?? "");
  const effort = effectiveEffort(props.tab);
  return effort ? `${name} (${effort})` : name;
}

function collaborationModeLabel(): string {
  if (props.tab.collaborationMode === "plan") return "计划模式";
  return "默认模式";
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
        @pointerdown="startComposerResize"
      >
        <span class="composer-resize-grip"></span>
      </div>
      <div class="composer-card">
        <div class="menu-anchor input-anchor">
          <EditorContent :editor="editor" class="rich-editor" @contextmenu.prevent />
          <MentionMenu
            ref="mentionMenu"
            v-if="mention"
            :kind="mention.kind"
            :token="mention.token"
            :results="fileResults"
            :searching="searchingFiles"
            :tab="props.tab"
            @close="mention = null"
            @pick-files="onPickFiles()"
            @pick-dir="onPickDir()"
            @select-file="onSelectFile($event)"
          />
        </div>
        <div class="composer-toolbar">
          <div class="composer-left">
            <div class="menu-anchor">
              <button
                class="perm-chip"
                v-tooltip="'权限模式'"
                :disabled="tab.turnActive"
                @click="toggleMenu('perm')"
              >
                <svg class="chip-icon" viewBox="0 0 24 24">
                  <path :d="permissionMode(tab.permissionMode).icon" />
                </svg>
                {{ permissionChip() }}
                <svg class="chevron" viewBox="0 0 16 16">
                  <path :d="ICON_CHEVRON_DOWN" />
                </svg>
              </button>
              <PermissionMenu v-if="permOpen" @close="permOpen = false" />
            </div>
            <div class="menu-anchor">
              <button
                class="collab-chip"
                v-tooltip="'协作模式'"
                :disabled="tab.turnActive"
                @click="toggleMenu('collab')"
              >
                <svg class="chip-icon" viewBox="0 0 24 24">
                  <path :d="collaborationMode(tab.collaborationMode).icon" />
                </svg>
                {{ collaborationModeLabel() }}
                <svg viewBox="0 0 16 16">
                <path :d="ICON_CHEVRON_DOWN" />
                </svg>
              </button>
              <CollaborationModeMenu v-if="collabOpen" @close="collabOpen = false" />
            </div>
            <div class="menu-anchor">
              <GoalChip :tab="tab" />
            </div>
          </div>
          <div class="composer-right">
            <ContextUsageMenu :tab="tab" />
            <div class="menu-anchor">
              <button
                class="model-chip"
                @click="toggleMenu('model')"
              >
                <svg class="model-chip-icon" viewBox="0 0 24 24">
                  <path :d="ICON_MODEL_CUBE" />
                </svg>
                {{ modelChipLabel() }}
                <svg viewBox="0 0 16 16">
                  <path :d="ICON_CHEVRON_DOWN" />
                </svg>
              </button>
              <ModelMenu v-if="modelOpen" @close="modelOpen = false" />
            </div>
            <button
              v-if="tab.turnActive"
              class="send-btn stop"
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
              :class="{ lit: !!(hasText || tab.attachments.length) }"
              :disabled="!hasText && tab.attachments.length === 0"
              @click="submit()"
            >
              <svg viewBox="0 0 24 24">
                <path d="M12 19V5M5 12l7-7 7 7" />
              </svg>
              发送
            </button>
          </div>
        </div>
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
          permOpen ||
          collabOpen ||
          modelOpen ||
          mention
        "
        class="menu-backdrop"
        @click="closeMenus()"
      ></div>
    </Teleport>
  </div>
</template>
