<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type {
  DiffEditorTab,
  DocxEditorTab,
  EditorTab,
  FileEditorTab,
  PreviewEditorTab,
} from "../composables/useEditorTabs";
import type { SessionTab } from "../composables/useCodex";
import { store } from "../composables/useCodex";
import { isThreadBound } from "../composables/useCodex/wechat";
import { ensureEntryIcons, iconFor } from "../composables/useSessionFs";
import { diffKindLabel } from "../lib/gitChanges";
import { relPathOf } from "../lib/format";
import type { FsEntry } from "../lib/sessionFs";
import { isTabWorking, TabIcon, TabKind } from "../lib/tabs";
import {
  ICON_CLOSE,
  ICON_FILE,
  ICON_GIT,
  ICON_SESSION,
  ICON_SETTINGS,
  ICON_TERMINAL,
  ICON_WECHAT,
} from "../lib/icons";

const props = defineProps<{
  sessionTabs: SessionTab[];
  editorTabs: EditorTab[];
  activeTabId: string;
}>();

const emit = defineEmits<{
  activate: [id: string];
  close: [tab: SessionTab | EditorTab];
  context: [e: MouseEvent, tab: SessionTab | EditorTab];
  add: [e: MouseEvent];
}>();

/** 终端标签内联重命名状态：renamingId 命中时标题原位变为输入框 */
const renamingId = ref("");
const renameDraft = ref("");

/** 进入终端标签标题重命名：预填当前标题并在渲染后聚焦输入框 */
function startRename(tab: EditorTab) {
  if (tab.kind !== TabKind.Terminal) return;
  renamingId.value = tab.id;
  renameDraft.value = tab.title;
  void nextTick(() => {
    document.querySelector<HTMLInputElement>(".tab-rename-input")?.focus();
  });
}

/** 提交重命名：trim 后非空且与原标题不同才写入（空值视为取消，保留原标题） */
function commitRename(tab: EditorTab) {
  if (renamingId.value !== tab.id) return;
  renamingId.value = "";
  const title = renameDraft.value.trim();
  if (title && title !== tab.title) {
    tab.title = title;
  }
}

function cancelRename() {
  renamingId.value = "";
}

/** 终端标签标题双击：进入内联重命名 */
function onLabelDblClick(tab: EditorTab) {
  if (tab.kind === TabKind.Terminal) startRename(tab);
}

/** 标签右键：输入框内放行给原生编辑菜单（不弹标签菜单） */
function onTabContext(e: MouseEvent, tab: SessionTab | EditorTab) {
  if ((e.target as HTMLElement).closest?.(".tab-rename-input")) return;
  emit("context", e, tab);
}

defineExpose({ startRename });

/** Tab 横向滚动：新标签/激活标签自动滚入视野，溢出时显示左右箭头 */
const tabScroller = ref<HTMLElement | null>(null);
const canScrollLeft = ref(false);
const canScrollRight = ref(false);

function updateTabScrollState() {
  const el = tabScroller.value;
  if (!el) return;
  canScrollLeft.value = el.scrollLeft > 2;
  canScrollRight.value = el.scrollLeft < el.scrollWidth - el.clientWidth - 2;
}

function scrollActiveTabIntoView() {
  const el = tabScroller.value;
  const activeEl = el?.querySelector<HTMLElement>(".editor-tab.active");
  if (!el || !activeEl) return;
  const pad = 8;
  const left = activeEl.offsetLeft;
  const right = left + activeEl.offsetWidth;
  if (left < el.scrollLeft) {
    el.scrollTo({ left: Math.max(0, left - pad), behavior: "smooth" });
  } else if (right > el.scrollLeft + el.clientWidth) {
    el.scrollTo({ left: right - el.clientWidth + pad, behavior: "smooth" });
  }
}

function scrollTabs(dir: -1 | 1) {
  const el = tabScroller.value;
  if (!el) return;
  el.scrollBy({
    left: dir * Math.max(120, Math.round(el.clientWidth * 0.7)),
    behavior: "smooth",
  });
}

watch(
  [() => props.activeTabId, () => props.sessionTabs.length + props.editorTabs.length],
  () => {
    void nextTick(() => {
      scrollActiveTabIntoView();
      updateTabScrollState();
    });
  },
);

onMounted(() => {
  window.addEventListener("resize", updateTabScrollState);
});
onBeforeUnmount(() => {
  window.removeEventListener("resize", updateTabScrollState);
});

/** 标签 → 伪 FsEntry，复用资源面板图标缓存/取图逻辑 */
function tabToEntry(
  tab: FileEditorTab | DocxEditorTab | DiffEditorTab | PreviewEditorTab,
): FsEntry {
  const root = tab.workspace;
  return {
    name: tab.title,
    path: tab.path,
    relPath: relPathOf(root, tab.path),
    isDir: false,
    size: null,
    modifiedAtMs: 0,
    createdAtMs: 0,
    childCount: null,
  };
}

function tabIcon(tab: EditorTab): string {
  if (
    tab.kind === TabKind.Terminal ||
    tab.kind === TabKind.Commit ||
    tab.kind === TabKind.Settings
  ) {
    return "";
  }
  return iconFor(tabToEntry(tab)) ?? "";
}

// 打开/关闭标签时按 root 分组懒加载缺失的文件图标（命中资源面板同一缓存）
watch(
  () =>
    props.editorTabs
      .filter((t) => t.kind !== TabKind.Terminal && t.kind !== TabKind.Commit)
      .map((t) => t.id),
  () => {
    const byRoot = new Map<string, FsEntry[]>();
    for (const tab of props.editorTabs) {
      if (
        tab.kind === TabKind.Terminal ||
        tab.kind === TabKind.Commit ||
        tab.kind === TabKind.Settings
      )
        continue;
      const t =
        tab as FileEditorTab | DocxEditorTab | DiffEditorTab | PreviewEditorTab;
      const list = byRoot.get(t.workspace) ?? [];
      list.push(tabToEntry(t));
      byRoot.set(t.workspace, list);
    }
    for (const [root, entries] of byRoot) {
      void ensureEntryIcons(entries, root);
    }
  },
  { immediate: true },
);

/** 会话标签待处理交互计数：绑定线程读标签记录，新对话（无线程）回退全局 */
function sessionTabPending(tab: SessionTab): number {
  return tab.threadId ? (tab.interactions?.length ?? 0) : store.interactions.length;
}

/**
 * 标题悬停提示：标题与路径不同时才显示路径，避免提示与可见标题重复。
 * 文件/diff/预览显示相对工作区根的路径；终端不显示 ToolTip。
 */
function titleTooltip(tab: EditorTab): string {
  if (tab.kind === TabKind.Settings) return "";
  if (tab.kind === TabKind.Terminal) return "";
  if (tab.kind === TabKind.Commit) return tab.hash;
  const path = relPathOf(tab.workspace, tab.path);
  return tab.title !== path ? path : "";
}
</script>

<template>
  <button
    v-if="canScrollLeft"
    class="editor-tab-scroll editor-tab-scroll-left"
    aria-label="向左滚动标签"
    @click="scrollTabs(-1)"
  >
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M15.41 7.41 14 6l-6 6 6 6 1.41-1.41L10.83 12z" />
    </svg>
  </button>
  <div class="editor-tabs-track" role="tablist" aria-label="编辑标签">
    <div
      ref="tabScroller"
      class="editor-tabs"
      @scroll.passive="updateTabScrollState"
    >
      <div
        v-for="tab in sessionTabs"
        :key="tab.id"
        class="editor-tab session-tab"
        :class="{ active: tab.id === activeTabId }"
        role="tab"
        :aria-selected="tab.id === activeTabId"
        :aria-label="tab.title"
        :tabindex="tab.id === activeTabId ? 0 : -1"
        @click="emit('activate', tab.id)"
        @keydown.enter.prevent="emit('activate', tab.id)"
        @keydown.space.prevent="emit('activate', tab.id)"
        @contextmenu="emit('context', $event, tab)"
        @mousedown.middle.prevent="emit('close', tab)"
      >
        <span class="editor-tab-logo" aria-hidden="true">
          <svg viewBox="0 0 24 24">
            <path
              :d="isThreadBound(tab.threadId) ? ICON_WECHAT : ICON_SESSION"
              fill="currentColor"
              stroke="none"
              :fill-rule="isThreadBound(tab.threadId) ? 'nonzero' : 'evenodd'"
            />
          </svg>
        </span>
        <span class="editor-tab-label">{{ tab.title }}</span>
        <span
          v-if="isTabWorking(tab)"
          class="editor-tab-run inline"
          aria-hidden="true"
        ></span>
        <span
          v-if="sessionTabPending(tab) > 0"
          class="interaction-badge"
          v-tooltip="`${sessionTabPending(tab)} 个待处理交互`"
        >
          {{ sessionTabPending(tab) }}
        </span>
        <button
          class="editor-tab-close"
          :aria-label="'关闭会话 ' + tab.title"
          @click.stop="emit('close', tab)"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_CLOSE" />
          </svg>
        </button>
      </div>
      <div
        v-for="tab in editorTabs"
        :key="tab.id"
        class="editor-tab"
        :class="{
          active: tab.id === activeTabId,
          'is-diff': tab.kind === TabKind.Diff,
        }"
        role="tab"
        :aria-selected="tab.id === activeTabId"
        :aria-label="tab.title"
        :tabindex="tab.id === activeTabId ? 0 : -1"
        @click="emit('activate', tab.id)"
        @keydown.enter.prevent="emit('activate', tab.id)"
        @keydown.space.prevent="emit('activate', tab.id)"
        @contextmenu="onTabContext($event, tab)"
        @mousedown.middle.prevent="emit('close', tab)"
      >
        <span
          v-if="tab.icon === TabIcon.Terminal"
          class="editor-tab-icon"
          aria-hidden="true"
        >
          <svg viewBox="0 0 24 24">
            <path :d="ICON_TERMINAL" />
          </svg>
        </span>
        <span
          v-else-if="tab.icon === TabIcon.Commit"
          class="editor-tab-icon"
          aria-hidden="true"
        >
          <svg viewBox="0 0 16 16">
            <path :d="ICON_GIT" />
          </svg>
        </span>
        <span
          v-else-if="tab.icon === TabIcon.Settings"
          class="editor-tab-icon"
          aria-hidden="true"
        >
          <svg viewBox="0 0 24 24">
            <path :d="ICON_SETTINGS" />
          </svg>
        </span>
        <span v-else class="editor-tab-icon" aria-hidden="true">
          <img
            v-if="tabIcon(tab)"
            class="editor-tab-icon-img"
            :src="tabIcon(tab)"
            alt=""
            draggable="false"
          />
          <svg v-else viewBox="0 0 24 24">
            <path :d="ICON_FILE" />
          </svg>
        </span>
        <span
          v-if="isTabWorking(tab)"
          class="editor-tab-run inline"
          aria-hidden="true"
        ></span>
        <input
          v-if="renamingId === tab.id"
          v-model="renameDraft"
          class="rename-input tab-rename-input"
          aria-label="重命名终端标题"
          @click.stop
          @keydown.enter="commitRename(tab)"
          @keydown.esc="cancelRename()"
          @blur="commitRename(tab)"
        />
        <span
          v-else
          class="editor-tab-label"
          v-tooltip="titleTooltip(tab)"
          @dblclick="onLabelDblClick(tab)"
        >
          {{ tab.title }}
        </span>
        <span
          v-if="
            (tab.kind === TabKind.File || tab.kind === TabKind.Docx) &&
            tab.dirty
          "
          class="editor-tab-dirty"
          v-tooltip="'未保存'"
        ></span>
        <span v-else-if="tab.kind === TabKind.Diff" class="editor-tab-kind">
          {{ diffKindLabel(tab.changeKind) }}
        </span>
        <span v-else-if="tab.kind === TabKind.Preview" class="editor-tab-kind">
          预览
        </span>
        <button
          class="editor-tab-close"
          :aria-label="'关闭 ' + tab.title"
          @click.stop="emit('close', tab)"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_CLOSE" />
          </svg>
        </button>
      </div>
      <button
        class="editor-tab-add"
        aria-label="新建会话或终端"
        v-tooltip="'新建会话 / 新建终端'"
        @click="emit('add', $event)"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6z" />
        </svg>
      </button>
    </div>
  </div>
  <button
    v-if="canScrollRight"
    class="editor-tab-scroll editor-tab-scroll-right"
    aria-label="向右滚动标签"
    @click="scrollTabs(1)"
  >
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M8.59 16.59 10 18l6-6-6-6-1.41 1.41L13.17 12z" />
    </svg>
  </button>
</template>
