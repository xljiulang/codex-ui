<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, watch } from "vue";
import ChatView from "./ChatView.vue";
import {
  activeTab,
  activeTabId,
  activateTab,
  cancelClose,
  closeAllOtherTabs,
  closeTab,
  discardTabAndClose,
  pendingCloseId,
  saveTabAndClose,
  tabs,
  type EditorTab,
  type FileEditorTab,
  type DiffEditorTab,
  type PreviewEditorTab,
} from "../composables/useEditorTabs";
import { ensureEntryIcons, iconFor } from "../composables/useSessionFs";
import type { FsEntry } from "../lib/sessionFs";
import { useActionMenu } from "../composables/useActionMenu";
import { setToast, store } from "../composables/useCodex";
import { relPathOf } from "../lib/format";
import { ICON_CLOSE_ALL } from "../lib/icons";

// CodeMirror / diff 渲染较重，仍按需加载，避免拖累主窗口首屏
const TextEditorPane = defineAsyncComponent(
  () => import("./TextEditorPane.vue"),
);
const DiffPane = defineAsyncComponent(() => import("./DiffPane.vue"));
const PreviewPane = defineAsyncComponent(() => import("./PreviewPane.vue"));

/** 通用文件回退图标（与资源面板一致） */
const ICON_FILE =
  "M14 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z";

const activeFileTab = computed(() =>
  activeTab.value?.kind === "file" ? activeTab.value : null,
);
const activeDiffTab = computed(() =>
  activeTab.value?.kind === "diff" ? activeTab.value : null,
);
const activePreviewTab = computed(() =>
  activeTab.value?.kind === "preview" ? activeTab.value : null,
);

/** 会话标签常驻，存在任何文件/diff 标签时才显示标签栏 */
const showTabBar = computed(() => tabs.length > 1);

/** 待关闭确认的脏文件标签 */
const pendingTab = computed<EditorTab | null>(
  () => tabs.find((t) => t.id === pendingCloseId.value) ?? null,
);

function kindLabel(kind: string): string {
  if (kind === "add") return "新增";
  if (kind === "delete") return "删除";
  return "修改";
}

/** 标签 → 伪 FsEntry，复用资源面板图标缓存/取图逻辑 */
function tabToEntry(tab: FileEditorTab | DiffEditorTab | PreviewEditorTab): FsEntry {
  const root =
    tab.kind === "file" ? tab.root : tab.kind === "diff" ? tab.workspaceRoot : tab.root;
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

function tabIcon(tab: FileEditorTab | DiffEditorTab | PreviewEditorTab): string {
  return iconFor(tabToEntry(tab)) ?? "";
}

/** 标签悬停提示：文件/diff 显示相对工作区根的路径 */
function tabTooltip(tab: EditorTab): string {
  if (tab.kind === "chat") {
    return store.turnActive ? "会话（进行中）" : "会话";
  }
  const root =
    tab.kind === "file" ? tab.root : tab.kind === "diff" ? tab.workspaceRoot : tab.root;
  return relPathOf(root, tab.path);
}

/** 会话主标签右键菜单：关闭其它所有标签（未保存的跳过） */
function openChatTabMenu(e: MouseEvent, tab: EditorTab) {
  if (tab.kind !== "chat" || tabs.length <= 1) return;
  openCtx(e, [
    {
      label: "关闭其它所有标签",
      icon: ICON_CLOSE_ALL,
      action: () => {
        const skipped = closeAllOtherTabs();
        if (skipped > 0) setToast(`已跳过 ${skipped} 个未保存的标签`);
      },
    },
  ]);
}

const { ctxMenu, openCtx, onWindowClick, onWindowScroll, onKeydown: onMenuKeydown } =
  useActionMenu({ width: 190, scrollScope: ".editor-tabs" });

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  onMenuKeydown(e);
}

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
  window.addEventListener("click", onWindowClick);
  window.addEventListener("scroll", onWindowScroll, true);
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("click", onWindowClick);
  window.removeEventListener("scroll", onWindowScroll, true);
});

// 打开/关闭标签时按 root 分组懒加载缺失的文件图标（命中资源面板同一缓存）
watch(
  () => tabs.filter((t) => t.kind !== "chat").map((t) => t.id),
  () => {
    const byRoot = new Map<string, FsEntry[]>();
    for (const tab of tabs) {
      if (tab.kind === "chat") continue;
      const root =
        tab.kind === "file" ? tab.root : tab.kind === "diff" ? tab.workspaceRoot : tab.root;
      const list = byRoot.get(root) ?? [];
      list.push(tabToEntry(tab));
      byRoot.set(root, list);
    }
    for (const [root, entries] of byRoot) {
      void ensureEntryIcons(entries, root);
    }
  },
  { immediate: true },
);
</script>

<template>
  <div class="editor-pane">
    <div v-if="showTabBar" class="editor-tabs" role="tablist" aria-label="编辑标签">
      <button
        v-for="tab in tabs"
        :key="tab.id"
        class="editor-tab"
        :class="{
          active: tab.id === activeTabId,
          pinned: tab.kind === 'chat',
          'is-diff': tab.kind === 'diff',
        }"
        role="tab"
        :aria-selected="tab.id === activeTabId"
        :aria-label="tab.kind === 'chat' ? '会话' : tab.title"
        :tabindex="tab.id === activeTabId ? 0 : -1"
        v-tooltip="tabTooltip(tab)"
        @click="activateTab(tab.id)"
        @contextmenu="openChatTabMenu($event, tab)"
        @mousedown.middle.prevent="closeTab(tab.id)"
      >
        <span v-if="tab.kind === 'chat'" class="editor-tab-logo" aria-hidden="true">
          <svg viewBox="0 0 24 24">
            <path d="M12 2l8.66 5v10L12 22l-8.66-5V7z" />
            <path class="logo-c" d="M14.9 9.1a4.5 4.5 0 1 0 0 5.8" />
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
          v-if="tab.kind === 'chat' && store.turnActive"
          class="editor-tab-run"
          aria-hidden="true"
        ></span>
        <span v-if="tab.kind !== 'chat'" class="editor-tab-label">
          {{ tab.title }}
        </span>
        <span
          v-if="tab.kind === 'file' && tab.dirty"
          class="editor-tab-dirty"
          title="未保存"
        ></span>
        <span v-else-if="tab.kind === 'diff'" class="editor-tab-kind">
          {{ kindLabel(tab.changeKind) }}
        </span>
        <span v-else-if="tab.kind === 'preview'" class="editor-tab-kind">预览</span>
        <button
          v-if="tab.kind !== 'chat'"
          class="editor-tab-close"
          :aria-label="'关闭 ' + tab.title"
          @click.stop="closeTab(tab.id)"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M6.4 5 5 6.4 10.6 12 5 17.6 6.4 19 12 13.4 17.6 19 19 17.6 13.4 12 19 6.4 17.6 5 12 10.6z"
            />
          </svg>
        </button>
      </button>
    </div>
    <div class="editor-pane-body">
      <ChatView v-show="activeTab?.kind === 'chat'" />
      <TextEditorPane v-if="activeFileTab" :tab="activeFileTab" />
      <DiffPane v-else-if="activeDiffTab" :tab="activeDiffTab" />
      <PreviewPane v-else-if="activePreviewTab" :tab="activePreviewTab" />
    </div>
    <div v-if="pendingTab" class="text-editor-overlay">
      <div class="text-editor-confirm">
        <div class="text-editor-confirm-msg">
          「{{ pendingTab.title }}」有未保存的更改，关闭将丢失这些更改。
        </div>
        <div class="text-editor-confirm-actions">
          <button
            class="text-editor-btn primary"
            @click="saveTabAndClose(pendingTab.id)"
          >
            保存并关闭
          </button>
          <button
            class="text-editor-btn"
            @click="discardTabAndClose(pendingTab.id)"
          >
            放弃并关闭
          </button>
          <button class="text-editor-btn" @click="cancelClose">取消</button>
        </div>
      </div>
    </div>
    <div
      v-if="ctxMenu"
      class="ctx-menu"
      :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }"
      @click.stop
    >
      <button
        v-for="it in ctxMenu.items"
        :key="it.label"
        class="ctx-menu-item"
        @click="it.action(); ctxMenu = null"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path :d="it.icon" fill-rule="evenodd" />
        </svg>
        <span>{{ it.label }}</span>
      </button>
    </div>
  </div>
</template>
