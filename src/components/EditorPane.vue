<script setup lang="ts">
import {
  computed,
  defineAsyncComponent,
  onBeforeUnmount,
  onMounted,
  watch,
} from "vue";
import ChatView from "./ChatView.vue";
import ContextMenu from "./ContextMenu.vue";
import EditorTabBar from "./EditorTabBar.vue";
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
import {
  revealAbsPathInTree,
  revealInExplorer,
} from "../composables/useSessionFs";
import { revealGitFile } from "../composables/useGitChanges";
import { joinFsPath } from "../lib/sessionFs";
import { useActionMenu, type CtxItem } from "../composables/useActionMenu";
import {
  newEmptyChat,
  setToast,
  store,
  workspace,
  type SessionTab,
} from "../composables/useCodex";
import { TabKind } from "../lib/tabs";
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

/** 待关闭确认的脏文件标签 */
const pendingTab = computed<EditorTab | null>(
  () =>
    (tabs.find((t) => t.id === pendingCloseId.value) as EditorTab | undefined) ??
    null,
);

/** 文件型标签（file/preview/diff）的磁盘绝对路径：兼容相对路径与工作区外绝对路径 */
function tabAbsPath(tab: FileEditorTab | DiffEditorTab | PreviewEditorTab): string {
  const root = tab.workspace;
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
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("click", onWindowClick);
  window.removeEventListener("scroll", onWindowScroll, true);
});

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
      <EditorTabBar
        :session-tabs="sessionTabs"
        :editor-tabs="editorTabs"
        :active-tab-id="activeTabId"
        :has-active-tab="hasActiveTab"
        @activate="activateTab"
        @close="closeAnyTab"
        @context="openTabMenu"
        @add="openAddMenu"
      />
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
    <ContextMenu
      v-if="ctxMenu"
      :items="ctxMenu.items"
      :x="ctxMenu.x"
      :y="ctxMenu.y"
      @close="ctxMenu = null"
    />
  </div>
</template>
