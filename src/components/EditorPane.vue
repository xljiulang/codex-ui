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
  closeAllTabs,
  closeAnyTab,
  closeTabsToLeftAll,
  closeTabsToRightAll,
  discardTabAndClose,
  openTerminalTab,
  pendingCloseId,
  saveTabAndClose,
  tabs,
  type EditorTab,
  type FileEditorTab,
  type DiffEditorTab,
  type PreviewEditorTab,
  type TerminalEditorTab,
} from "../composables/useEditorTabs";
import { diffKindLabel } from "../lib/gitChanges";
import {
  ensureEntryIcons,
  iconFor,
  revealAbsPathInTree,
  revealInExplorer,
} from "../composables/useSessionFs";
import { revealGitFile } from "../composables/useGitChanges";
import { joinFsPath, type FsEntry } from "../lib/sessionFs";
import { useActionMenu, type CtxItem } from "../composables/useActionMenu";
import {
  newEmptyChat,
  setToast,
  store,
  workspace,
  type SessionTab,
} from "../composables/useCodex";
import { isTabWorking, TabIcon, TabKind } from "../lib/tabs";
import { relPathOf } from "../lib/format";
import {
  ICON_CLOSE_ALL,
  ICON_CLOSE_LEFT,
  ICON_CLOSE_RIGHT,
  ICON_REVEAL,
  SESSION_LOGO_PATHS,
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

const activeFileTab = computed<FileEditorTab | null>(() =>
  activeTab.value?.kind === TabKind.File
    ? (activeTab.value as FileEditorTab)
    : null,
);
const activeDiffTab = computed<DiffEditorTab | null>(() =>
  activeTab.value?.kind === TabKind.Diff
    ? (activeTab.value as DiffEditorTab)
    : null,
);
const activePreviewTab = computed<PreviewEditorTab | null>(() =>
  activeTab.value?.kind === TabKind.Preview
    ? (activeTab.value as PreviewEditorTab)
    : null,
);
/** 终端标签列表：全部常驻挂载（v-show 切换），切走不销毁 xterm/不中断进程 */
const activeTerminalTabs = computed(() =>
  tabs.filter((t): t is TerminalEditorTab => t.kind === "terminal"),
);

/** 会话标签视图（统一列表中的会话块，恒在前） */
const sessionTabs = computed(() =>
  tabs.filter((t): t is SessionTab => t.kind === TabKind.Chat),
);
/** 文件/diff/预览/终端标签视图（统一列表中的其余部分，会话之后） */
const editorTabs = computed<EditorTab[]>(() =>
  tabs.filter((t): t is EditorTab => t.kind !== TabKind.Chat),
);

/** 标签栏常驻显示：所有标签关闭后仍保留「+」新建入口 */
const showTabBar = computed(() => true);

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
  () =>
    (tabs.find((t) => t.id === pendingCloseId.value) as EditorTab | undefined) ??
    null,
);

/** 标签 → 伪 FsEntry，复用资源面板图标缓存/取图逻辑 */
function tabToEntry(tab: FileEditorTab | DiffEditorTab | PreviewEditorTab): FsEntry {
  const root =
    tab.kind === TabKind.File ? tab.workspace : tab.kind === TabKind.Diff ? tab.workspace : tab.workspace;
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
  if (tab.kind === TabKind.Terminal) return "";
  return iconFor(tabToEntry(tab)) ?? "";
}

/** 会话标签悬停提示：进行中显示“会话（进行中）” */
function sessionTabTooltip(tab: SessionTab): string {
  return isTabWorking(tab) ? "会话（进行中）" : "会话";
}

/** 会话标签待处理交互计数：绑定线程读标签记录，新对话（无线程）回退全局 */
function sessionTabPending(tab: SessionTab): number {
  return tab.threadId ? (tab.interactions?.length ?? 0) : store.interactions.length;
}

/**
 * 标题悬停提示：标题与路径不同时才显示路径，避免提示与可见标题重复。
 * 文件/diff/预览显示相对工作区根的路径；终端不显示 ToolTip。
 */
function titleTooltip(tab: EditorTab): string {
  if (tab.kind === TabKind.Terminal) return "";
  const root =
    tab.kind === TabKind.File ? tab.workspace : tab.kind === TabKind.Diff ? tab.workspace : tab.workspace;
  const path = relPathOf(root, tab.path);
  return tab.title !== path ? path : "";
}

/** 文件型标签（file/preview/diff）的磁盘绝对路径：兼容相对路径与工作区外绝对路径 */
function tabAbsPath(tab: FileEditorTab | DiffEditorTab | PreviewEditorTab): string {
  const root =
    tab.kind === TabKind.File
      ? tab.workspace
      : tab.kind === TabKind.Diff
        ? tab.workspace
        : tab.workspace;
  return /^[A-Za-z]:[\\/]/.test(tab.path) ? tab.path : joinFsPath(root, tab.path);
}

/** 文件型标签（file/preview）的磁盘绝对路径：兼容工作区内绝对路径与外部文件（root=父目录+文件名） */
function fileTabAbsPath(tab: FileEditorTab | PreviewEditorTab): string {
  return tabAbsPath(tab);
}

/** 关闭所有标签（文件/diff/预览/终端 + 会话标签）：运行中会话/终端与未保存文件跳过并计数 */
async function closeAllTabsWithToast() {
  const skipped = await closeAllTabs();
  if (skipped > 0) {
    setToast(`已跳过 ${skipped} 个标签（未保存文件 / 运行中的终端 / 运行中的会话）`);
  }
}

/** 统一标签右键菜单：关闭所有 + 关闭左边/右边（按统一列表整体顺序）+ 文件直达目录 */
function openTabMenu(e: MouseEvent, tab: SessionTab | EditorTab) {
  const idx = tabs.findIndex((t) => t.id === tab.id);
  const hasLeft = idx > 0;
  const hasRight = idx >= 0 && idx < tabs.length - 1;
  const items: CtxItem[] = [
    {
      label: "关闭所有标签",
      icon: ICON_CLOSE_ALL,
      action: () => void closeAllTabsWithToast(),
    },
  ];
  if (hasLeft) {
    items.push({
      label: "关闭左边所有标签",
      icon: ICON_CLOSE_LEFT,
      action: () => {
        void closeTabsToLeftAll(tab.id).then((skipped) => {
          if (skipped > 0) {
            setToast(
              `已跳过 ${skipped} 个标签（未保存文件 / 运行中的终端 / 运行中的会话）`,
            );
          }
        });
      },
    });
  }
  if (hasRight) {
    items.push({
      label: "关闭右边所有标签",
      icon: ICON_CLOSE_RIGHT,
      action: () => {
        void closeTabsToRightAll(tab.id).then((skipped) => {
          if (skipped > 0) {
            setToast(
              `已跳过 ${skipped} 个标签（未保存文件 / 运行中的终端 / 运行中的会话）`,
            );
          }
        });
      },
    });
  }
  // 文件/预览标签（含对话打开的工作区外文件）可直达所在目录，置于菜单末尾
  if (tab.kind === TabKind.File || tab.kind === TabKind.Preview) {
    items.push({
      label: "在资源管理器中打开",
      icon: ICON_REVEAL,
      action: () => revealInExplorer(fileTabAbsPath(tab)),
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
      .filter((t) => t.kind !== TabKind.Terminal && t.kind !== TabKind.Chat)
      .map((t) => t.id),
  () => {
    const byRoot = new Map<string, FsEntry[]>();
    for (const tab of tabs) {
      if (tab.kind === TabKind.Terminal || tab.kind === TabKind.Chat) continue;
      const t = tab as FileEditorTab | DiffEditorTab | PreviewEditorTab;
      const root =
        t.kind === TabKind.File
          ? t.workspace
          : t.kind === TabKind.Diff
            ? t.workspace
            : t.workspace;
      const list = byRoot.get(root) ?? [];
      list.push(tabToEntry(t));
      byRoot.set(root, list);
    }
    for (const [root, entries] of byRoot) {
      void ensureEntryIcons(entries, root);
    }
  },
  { immediate: true },
);

/**
 * Tab 激活：把活动标签的工作区写入 store.workspace（资源/Git 面板跟随切换），
 * 带文件路径的标签（文件/预览/Diff）再在资源树中同步选中并展开所在目录。
 */
watch(activeTab, (tab) => {
  store.workspace = tab ? tab.workspace : null;
  if (tab && tab.kind === TabKind.Diff) {
    revealGitFile(tab.workspace, tab.path);
  }
  if (!tab || tab.kind === TabKind.Terminal || tab.kind === TabKind.Chat) return;
  void revealAbsPathInTree(tabAbsPath(tab));
});

/** 活动标签的工作区（会话/文件/diff/预览/终端统一取 tab.workspace） */
const activeWorkspace = computed((): string | null => activeTab.value?.workspace ?? null);

/**
 * 是否存在激活状态的标签（无活动标签时隐藏「+」）。
 * 按“活动标签真实存在”判定（activeTab 覆盖统一列表全部类型）。
 */
const hasActiveTab = computed(
  () => activeTabId.value !== "" && activeTab.value !== null,
);

/** 标签栏末尾「+」：选择新建会话或新建终端，均使用活动标签工作区启动 */
function openAddMenu(e: MouseEvent) {
  const ws = activeWorkspace.value || workspace.value || "";
  openCtx(e, [
    {
      label: "新建会话",
      paths: SESSION_LOGO_PATHS,
      action: () => void newEmptyChat(ws || null),
    },
    {
      label: "新建终端",
      icon: ICON_TERMINAL,
      action: () => void openTerminalTab(ws),
    },
  ]);
}

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
        <div
          ref="tabScroller"
          class="editor-tabs"
          @scroll.passive="updateTabScrollState"
        >
          <button
            v-for="tab in sessionTabs"
            :key="tab.id"
            class="editor-tab session-tab"
            :class="{ active: tab.id === activeTabId }"
            role="tab"
            :aria-selected="tab.id === activeTabId"
            :aria-label="tab.title"
            :tabindex="tab.id === activeTabId ? 0 : -1"
            v-tooltip="sessionTabTooltip(tab)"
            @click="activateTab(tab.id)"
            @contextmenu="openTabMenu($event, tab)"
            @mousedown.middle.prevent="closeAnyTab(tab)"
          >
            <span class="editor-tab-logo" aria-hidden="true">
              <svg viewBox="0 0 24 24">
                <path d="M12 2l8.66 5v10L12 22l-8.66-5V7z" />
                <path class="logo-c" d="M14.9 9.1a4.5 4.5 0 1 0 0 5.8" />
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
              :title="`${sessionTabPending(tab)} 个待处理交互`"
            >
              {{ sessionTabPending(tab) }}
            </span>
            <button
              class="editor-tab-close"
              :aria-label="'关闭会话 ' + tab.title"
              @click.stop="closeAnyTab(tab)"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path
                  d="M6.4 5 5 6.4 10.6 12 5 17.6 6.4 19 12 13.4 17.6 19 19 17.6 13.4 12 19 6.4 17.6 5 12 10.6z"
                />
              </svg>
            </button>
          </button>
          <button
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
            @click="activateTab(tab.id)"
            @contextmenu="openTabMenu($event, tab)"
            @mousedown.middle.prevent="closeAnyTab(tab)"
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
            <span class="editor-tab-label" v-tooltip="titleTooltip(tab)">
              {{ tab.title }}
            </span>
            <span
              v-if="tab.kind === TabKind.File && tab.dirty"
              class="editor-tab-dirty"
              title="未保存"
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
              @click.stop="closeAnyTab(tab)"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path
                  d="M6.4 5 5 6.4 10.6 12 5 17.6 6.4 19 12 13.4 17.6 19 19 17.6 13.4 12 19 6.4 17.6 5 12 10.6z"
                />
              </svg>
            </button>
          </button>
          <button
            v-if="hasActiveTab"
            class="editor-tab-add"
            aria-label="新建会话或终端"
            v-tooltip="'新建会话 / 新建终端'"
            @click="openAddMenu($event)"
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
    </div>
    <div class="editor-pane-body">
      <div v-if="tabs.length === 0" class="no-session-state">
        <div class="empty-logo">
          <svg viewBox="0 0 24 24">
            <path d="M12 2l8.66 5v10L12 22l-8.66-5V7z" />
            <path class="logo-c" d="M14.9 9.1a4.5 4.5 0 1 0 0 5.8" />
          </svg>
        </div>
        <p class="no-session-hint">当前还没有任何打开的项</p>
      </div>
      <ChatView
        v-for="tab in sessionTabs"
        :key="tab.id"
        v-show="activeTabId === tab.id"
        :tab="tab"
        :active="activeTabId === tab.id"
      />
      <TextEditorPane v-if="activeFileTab" :tab="activeFileTab" />
      <DiffPane v-else-if="activeDiffTab" :tab="activeDiffTab" />
      <PreviewPane v-else-if="activePreviewTab" :tab="activePreviewTab" />
      <TerminalPane
        v-for="t in activeTerminalTabs"
        :key="t.id"
        v-show="activeTabId === t.id"
        :tab="t"
        :active="activeTabId === t.id"
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
        <svg
          viewBox="0 0 24 24"
          aria-hidden="true"
          :class="{ 'ctx-session-logo': it.paths }"
        >
          <template v-if="it.paths">
            <path
              v-for="p in it.paths"
              :key="p.d"
              :d="p.d"
              :class="{ 'logo-c': p.accent }"
            />
          </template>
          <path v-else :d="it.icon" fill-rule="evenodd" />
        </svg>
        <span>{{ it.label }}</span>
      </button>
    </div>
  </div>
</template>
