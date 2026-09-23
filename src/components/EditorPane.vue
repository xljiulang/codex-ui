<script setup lang="ts">
import {
  computed,
  defineAsyncComponent,
  onBeforeUnmount,
  onMounted,
  ref,
  watch,
} from "vue";
import ChatView from "./ChatView.vue";
import ContextMenu from "./ContextMenu.vue";
import EditorTabBar from "./EditorTabBar.vue";
import SettingsView from "./SettingsView.vue";
import { invoke } from "@tauri-apps/api/core";
import {
  activeTab,
  activeTabId,
  activateTab,
  cancelClose,
  closeAllTabs,
  closeAnyTab,
  closeOtherTabs,
  closeTabsToLeftAll,
  closeTabsToRightAll,
  discardTabAndClose,
  openTerminalTab,
  pendingCloseId,
  saveTabAndClose,
  tabs,
  type EditorTab,
  type CommitEditorTab,
  type FileEditorTab,
  type DiffEditorTab,
  type PreviewEditorTab,
  type TerminalEditorTab,
  SETTINGS_TAB_ID,
} from "../composables/useEditorTabs";
import {
  openPathInApp,
  revealAbsPathInTree,
  revealInExplorer,
} from "../composables/useSessionFs";
import { revealGitFile } from "../composables/useGitChanges";
import { pathBaseName } from "../lib/format";
import { joinFsPath } from "../lib/sessionFs";
import {
  useActionMenu,
  type ActionMenuItem,
} from "../composables/useActionMenu";
import {
  pickAndOpenNewSession,
  setToast,
  toastError,
  store,
  workspace,
  type SessionTab,
} from "../composables/useCodex";
import { TabKind } from "../lib/tabs";
import {
  ICON_CLOSE_ALL,
  ICON_CLOSE_LEFT,
  ICON_CLOSE_OTHERS,
  ICON_CLOSE_RIGHT,
  ICON_EXTERNAL_LINK,
  ICON_OPEN,
  ICON_RENAME,
  ICON_SESSION,
  ICON_TERMINAL,
} from "../lib/icons";

// CodeMirror / diff 渲染较重，仍按需加载，避免拖累主窗口首屏
const TextEditorPane = defineAsyncComponent(
  () => import("./TextEditorPane.vue"),
);
const DiffPane = defineAsyncComponent(() => import("./DiffPane.vue"));
const PreviewPane = defineAsyncComponent(() => import("./PreviewPane.vue"));
const TerminalPane = defineAsyncComponent(() => import("./TerminalPane.vue"));
const CommitPane = defineAsyncComponent(() => import("./CommitPane.vue"));

/** EditorTabBar 实例引用：右键菜单「重命名」调用其暴露的内联重命名入口 */
const tabBarRef = ref<InstanceType<typeof EditorTabBar> | null>(null);

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
const activeCommitTab = computed<CommitEditorTab | null>(() =>
  activeTab.value?.kind === TabKind.Commit
    ? (activeTab.value as CommitEditorTab)
    : null,
);
/** 终端标签列表：全部常驻挂载（v-show 切换），切走不销毁 xterm/不中断进程 */
const activeTerminalTabs = computed(() =>
  tabs.filter((t): t is TerminalEditorTab => t.kind === "terminal"),
);

/** 会话标签视图（统一列表中的会话块，恒在前） */
const sessionTabs = computed(() =>
  tabs.filter((t): t is SessionTab => t.kind === TabKind.Session),
);
/** 文件/diff/预览/终端标签视图（统一列表中的其余部分，会话之后） */
const editorTabs = computed<EditorTab[]>(() =>
  tabs.filter((t): t is EditorTab => t.kind !== TabKind.Session),
);

/** 待关闭确认的脏文件标签 */
const pendingTab = computed<EditorTab | null>(
  () =>
    (tabs.find((t) => t.id === pendingCloseId.value) as
      EditorTab | undefined) ?? null,
);

/** 文件型标签（file/preview/diff）的磁盘绝对路径：兼容相对路径与工作区外绝对路径 */
function tabAbsPath(
  tab: FileEditorTab | DiffEditorTab | PreviewEditorTab,
): string {
  const root = tab.workspace;
  return /^[A-Za-z]:[\\/]/.test(tab.path)
    ? tab.path
    : joinFsPath(root, tab.path);
}

/** 文件型标签（file/preview）的磁盘绝对路径：兼容工作区内绝对路径与外部文件（root=父目录+文件名） */
function fileTabAbsPath(tab: FileEditorTab | PreviewEditorTab): string {
  return tabAbsPath(tab);
}

/** 批量关闭跳过提示：运行中会话/终端与未保存文件不逐个确认，仅计数 */
function reportSkipped(skipped: number) {
  if (skipped > 0) {
    setToast(
      `已跳过 ${skipped} 个标签（未保存文件 / 运行中的终端 / 运行中的会话）`,
    );
  }
}

/** 关闭所有标签（文件/diff/预览/终端 + 会话标签） */
async function closeAllTabsWithToast() {
  reportSkipped(await closeAllTabs());
}

/** 统一标签右键菜单：关闭所有/其它 + 关闭左边/右边（按统一列表整体顺序）+ 文件直达目录 */
function openTabMenu(e: MouseEvent, tab: SessionTab | EditorTab) {
  const idx = tabs.findIndex((t) => t.id === tab.id);
  const hasLeft = idx > 0;
  const hasRight = idx >= 0 && idx < tabs.length - 1;
  const items: ActionMenuItem[] = [
    {
      label: "关闭所有标签",
      icon: ICON_CLOSE_ALL,
      action: () => void closeAllTabsWithToast(),
    },
  ];
  if (tabs.length > 1) {
    items.push({
      label: "关闭其它标签",
      icon: ICON_CLOSE_OTHERS,
      action: () => {
        void closeOtherTabs(tab.id).then(reportSkipped);
      },
    });
  }
  if (hasLeft) {
    items.push({
      label: "关闭左边所有标签",
      icon: ICON_CLOSE_LEFT,
      action: () => {
        void closeTabsToLeftAll(tab.id).then(reportSkipped);
      },
    });
  }
  if (hasRight) {
    items.push({
      label: "关闭右边所有标签",
      icon: ICON_CLOSE_RIGHT,
      action: () => {
        void closeTabsToRightAll(tab.id).then(reportSkipped);
      },
    });
  }
  // 终端标签：标题处内联重命名（双击标题或右键菜单均可进入）
  if (tab.kind === TabKind.Terminal) {
    items.push({
      label: "重命名",
      icon: ICON_RENAME,
      action: () => tabBarRef.value?.startRename(tab as TerminalEditorTab),
    });
  }
  // 文件/预览标签（含对话打开的工作区外文件）可直达所在目录，置于菜单末尾
  if (tab.kind === TabKind.File || tab.kind === TabKind.Preview) {
    items.push({
      label: "在资源管理器中打开",
      icon: ICON_EXTERNAL_LINK,
      action: () => revealInExplorer(fileTabAbsPath(tab)),
    });
  }
  openCtx(e, items);
}

const {
  ctxMenu,
  openCtx,
  onWindowClick,
  onWindowScroll,
  onKeydown: onMenuKeydown,
} = useActionMenu({ width: 190, scrollScope: ".editor-tabs-bar" });

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
  if (
    tab &&
    (tab.kind === TabKind.File ||
      tab.kind === TabKind.Preview ||
      tab.kind === TabKind.Diff)
  ) {
    revealGitFile(tab.workspace, tab.path);
  }
  if (
    !tab ||
    tab.kind === TabKind.Terminal ||
    tab.kind === TabKind.Session ||
    tab.kind === TabKind.Commit ||
    tab.kind === TabKind.Settings
  ) {
    return;
  }
  void revealAbsPathInTree(tabAbsPath(tab));
});

/** 设置标签是否存在：存在期间常驻挂载（v-show 切换），关闭后销毁重置 */
const settingsTabOpen = computed(() =>
  tabs.some((t) => t.id === SETTINGS_TAB_ID),
);

/** 标签栏末尾「+」菜单：新建会话走与头部一致的工作目录选择；
 * 打开文件走原生文件选择器（多次选择，应用内打开；不支持的类型提示无法打开）；
 * 新建终端拆为 cmd / PowerShell 两项，按所选 Shell 强制启动 */
/** 打开文件流程：文件选择器多选 → 逐路径应用内打开；不支持的类型提示无法打开 */
async function pickAndOpenFile() {
  const ws = workspace.value || store.lastWorkspace || "";
  try {
    const picked = await invoke<string[]>("pick_files", {
      multiple: true,
      initialDir: ws || null,
    });
    for (const p of picked) {
      const opened = await openPathInApp(p);
      if (opened) continue;
      setToast(`该文件不是文本文件，无法打开：${pathBaseName(p)}`);
    }
  } catch (e) {
    setToast(toastError(e));
  }
}

function openAddMenu(e: MouseEvent) {
  const ws = workspace.value || store.lastWorkspace || "";
  const items: ActionMenuItem[] = [
    {
      label: "新建会话",
      icon: ICON_SESSION,
      action: () => void pickAndOpenNewSession(),
    },
    {
      label: "打开文件",
      icon: ICON_OPEN,
      action: () => void pickAndOpenFile(),
    },
    {
      label: "新建终端(cmd)",
      icon: ICON_TERMINAL,
      action: () => void openTerminalTab(ws, "cmd"),
    },
    {
      label: "新建终端(PowerShell)",
      icon: ICON_TERMINAL,
      action: () => void openTerminalTab(ws, "powershell"),
    },
  ];
  openCtx(e, items);
}
</script>

<template>
  <div class="editor-pane">
    <div class="editor-tabs-bar">
      <EditorTabBar
        ref="tabBarRef"
        :session-tabs="sessionTabs"
        :editor-tabs="editorTabs"
        :active-tab-id="activeTabId"
        @activate="activateTab"
        @close="closeAnyTab"
        @context="openTabMenu"
        @add="openAddMenu"
      />
    </div>
    <div class="editor-pane-body">
      <div v-if="tabs.length === 0" class="no-tabs-state">
        <div class="empty-logo">
          <svg viewBox="0 0 24 24">
            <path d="M12 2l8.66 5v10L12 22l-8.66-5V7z" />
            <path class="logo-c" d="M14.9 9.1a4.5 4.5 0 1 0 0 5.8" />
          </svg>
        </div>
        <p class="no-tabs-hint">当前还没有任何打开的项</p>
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
      <CommitPane v-else-if="activeCommitTab" :tab="activeCommitTab" />
      <TerminalPane
        v-for="t in activeTerminalTabs"
        :key="t.id"
        v-show="activeTabId === t.id"
        :tab="t"
        :active="activeTabId === t.id"
      />
      <SettingsView
        v-if="settingsTabOpen"
        v-show="activeTabId === SETTINGS_TAB_ID"
      />
    </div>
    <div v-if="pendingTab" class="tab-confirm-overlay">
      <div class="tab-confirm">
        <div class="tab-confirm-msg">
          「{{ pendingTab.title }}」有未保存的更改，关闭将丢失这些更改。
        </div>
        <div class="tab-confirm-actions">
          <button
            class="btn primary tab-confirm-btn"
            @click="saveTabAndClose(pendingTab.id)"
          >
            保存并关闭
          </button>
          <button
            class="btn tab-confirm-btn"
            @click="discardTabAndClose(pendingTab.id)"
          >
            放弃并关闭
          </button>
          <button class="btn tab-confirm-btn" @click="cancelClose">取消</button>
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
