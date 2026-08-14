<script setup lang="ts">
import {
  computed,
  defineAsyncComponent,
  nextTick,
  onBeforeUnmount,
  onMounted,
  ref,
  watch,
} from "vue";
import ChatView from "./ChatView.vue";
import {
  activeTab,
  activeTabId,
  activateTab,
  cancelClose,
  closeAllOtherTabs,
  closeTabsToLeft,
  closeTabsToRight,
  closeTab,
  discardTabAndClose,
  pendingCloseId,
  saveTabAndClose,
  tabs,
  type EditorTab,
  type ChatEditorTab,
  type FileEditorTab,
  type DiffEditorTab,
  type PreviewEditorTab,
  type TerminalEditorTab,
} from "../composables/useEditorTabs";
import { ensureEntryIcons, iconFor } from "../composables/useSessionFs";
import type { FsEntry } from "../lib/sessionFs";
import { useActionMenu, type CtxItem } from "../composables/useActionMenu";
import { setToast, store } from "../composables/useCodex";
import { relPathOf } from "../lib/format";
import {
  ICON_CLOSE_ALL,
  ICON_CLOSE_LEFT,
  ICON_CLOSE_RIGHT,
  ICON_TERMINAL,
} from "../lib/icons";

// CodeMirror / diff 渲染较重，仍按需加载，避免拖累主窗口首屏
const TextEditorPane = defineAsyncComponent(
  () => import("./TextEditorPane.vue"),
);
const DiffPane = defineAsyncComponent(() => import("./DiffPane.vue"));
const PreviewPane = defineAsyncComponent(() => import("./PreviewPane.vue"));
const TerminalPane = defineAsyncComponent(() => import("./TerminalPane.vue"));

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
/** 终端标签列表：全部常驻挂载（v-show 切换），切走不销毁 xterm/不中断进程 */
const activeTerminalTabs = computed(() =>
  tabs.filter((t): t is TerminalEditorTab => t.kind === "terminal"),
);

/** 会话主标签：常驻标签栏左侧固定展示（滚动区外），不可关闭 */
const chatTab = computed<ChatEditorTab>(
  () => tabs.find((t): t is ChatEditorTab => t.kind === "chat")!,
);
/** 会话以外的标签：在右侧独立滚动区内滚动 */
const otherTabs = computed(() => tabs.filter((t) => t.kind !== "chat"));

/** 会话标签常驻，存在任何文件/diff 标签时才显示标签栏 */
const showTabBar = computed(() => tabs.length > 1);

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
  [activeTabId, () => tabs.length],
  () => {
    void nextTick(() => {
      scrollActiveTabIntoView();
      updateTabScrollState();
    });
  },
);

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

/** 标签头悬停提示：仅会话标签保留（非会话标签的提示移到标题上） */
function tabTooltip(tab: EditorTab): string {
  if (tab.kind === "chat") {
    return store.turnActive ? "会话（进行中）" : "会话";
  }
  return "";
}

/**
 * 标题悬停提示：标题与路径不同时才显示路径，避免提示与可见标题重复。
 * 文件/diff/预览显示相对工作区根的路径，终端显示工作目录。
 */
function titleTooltip(tab: EditorTab): string {
  if (tab.kind === "chat") return "";
  if (tab.kind === "terminal") {
    return tab.title !== tab.cwd ? tab.cwd : "";
  }
  const root =
    tab.kind === "file" ? tab.root : tab.kind === "diff" ? tab.workspaceRoot : tab.root;
  const path = relPathOf(root, tab.path);
  return tab.title !== path ? path : "";
}

/**
 * 标签右键菜单：所有标签统一提供关闭所有/左边/右边（未保存的跳过并提示）。
 * 会话主标签固定在首位且不可关闭：左边项天然隐藏（idx=0），关闭所有天然
 * 排除会话标签（closeAllOtherTabs），无需特判。
 */
function openTabMenu(e: MouseEvent, tab: EditorTab) {
  const closeWithToast = (skipped: number) => {
    if (skipped > 0) setToast(`已跳过 ${skipped} 个未保存的标签`);
  };
  const idx = tabs.findIndex((t) => t.id === tab.id);
  const hasLeft = tabs.slice(0, idx).some((t) => t.kind !== "chat");
  const hasRight = tabs.slice(idx + 1).some((t) => t.kind !== "chat");
  const items: CtxItem[] = [
    {
      label: "关闭所有标签",
      icon: ICON_CLOSE_ALL,
      action: () => closeWithToast(closeAllOtherTabs()),
    },
  ];
  if (hasLeft) {
    items.push({
      label: "关闭左边所有标签",
      icon: ICON_CLOSE_LEFT,
      action: () => closeWithToast(closeTabsToLeft(tab.id)),
    });
  }
  if (hasRight) {
    items.push({
      label: "关闭右边所有标签",
      icon: ICON_CLOSE_RIGHT,
      action: () => closeWithToast(closeTabsToRight(tab.id)),
    });
  }
  openCtx(e, items);
}

const { ctxMenu, openCtx, onWindowClick, onWindowScroll, onKeydown: onMenuKeydown } =
  useActionMenu({ width: 190, scrollScope: ".editor-tabs-bar" });

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  onMenuKeydown(e);
}

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
  window.addEventListener("click", onWindowClick);
  window.addEventListener("scroll", onWindowScroll, true);
  window.addEventListener("resize", updateTabScrollState);
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("click", onWindowClick);
  window.removeEventListener("scroll", onWindowScroll, true);
  window.removeEventListener("resize", updateTabScrollState);
});

// 打开/关闭标签时按 root 分组懒加载缺失的文件图标（命中资源面板同一缓存）
watch(
  () =>
    tabs
      .filter((t) => t.kind !== "chat" && t.kind !== "terminal")
      .map((t) => t.id),
  () => {
    const byRoot = new Map<string, FsEntry[]>();
    for (const tab of tabs) {
      if (tab.kind === "chat" || tab.kind === "terminal") continue;
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
    <div v-if="showTabBar" class="editor-tabs-bar">
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
        <button
          class="editor-tab pinned"
          :class="{ active: activeTabId === 'chat' }"
          role="tab"
          :aria-selected="activeTabId === 'chat'"
          aria-label="会话"
          :tabindex="activeTabId === 'chat' ? 0 : -1"
          v-tooltip="tabTooltip(chatTab)"
          @click="activateTab('chat')"
          @contextmenu="openTabMenu($event, chatTab)"
          @mousedown.middle.prevent="closeTab('chat')"
        >
          <span class="editor-tab-logo" aria-hidden="true">
            <svg viewBox="0 0 24 24">
              <path d="M12 2l8.66 5v10L12 22l-8.66-5V7z" />
              <path class="logo-c" d="M14.9 9.1a4.5 4.5 0 1 0 0 5.8" />
            </svg>
          </span>
          <span
            v-if="store.turnActive"
            class="editor-tab-run"
            aria-hidden="true"
          ></span>
        </button>
        <div
          ref="tabScroller"
          class="editor-tabs"
          @scroll.passive="updateTabScrollState"
        >
          <button
            v-for="tab in otherTabs"
            :key="tab.id"
            class="editor-tab"
            :class="{
              active: tab.id === activeTabId,
              'is-diff': tab.kind === 'diff',
            }"
            role="tab"
            :aria-selected="tab.id === activeTabId"
            :aria-label="tab.title"
            :tabindex="tab.id === activeTabId ? 0 : -1"
            v-tooltip="tabTooltip(tab)"
            @click="activateTab(tab.id)"
            @contextmenu="openTabMenu($event, tab)"
            @mousedown.middle.prevent="closeTab(tab.id)"
          >
            <span
              v-if="tab.kind === 'terminal'"
              class="editor-tab-icon"
              aria-hidden="true"
            >
              <svg viewBox="0 0 24 24">
                <path :d="ICON_TERMINAL" />
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
            <span class="editor-tab-label" v-tooltip="titleTooltip(tab)">
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
            <span v-else-if="tab.kind === 'preview'" class="editor-tab-kind">
              预览
            </span>
            <button
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
    </div>
    <div class="editor-pane-body">
      <ChatView v-show="activeTab?.kind === 'chat'" />
      <TextEditorPane v-if="activeFileTab" :tab="activeFileTab" />
      <DiffPane v-else-if="activeDiffTab" :tab="activeDiffTab" />
      <PreviewPane v-else-if="activePreviewTab" :tab="activePreviewTab" />
      <TerminalPane
        v-for="t in activeTerminalTabs"
        :key="t.id"
        v-show="activeTabId === t.id"
        :tab="t"
      />
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
