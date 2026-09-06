<script setup lang="ts">
import ContextMenu from "./ContextMenu.vue";
import ModalDialog from "./ModalDialog.vue";
import WechatBindDialog from "./WechatBindDialog.vue";
import {
  computed,
  onBeforeUnmount,
  onMounted,
  reactive,
  ref,
  watch,
} from "vue";
import { invoke } from "@tauri-apps/api/core";
import { openTerminalTab } from "../composables/useEditorTabs";
import {
  activeSessionTab,
  clearSearch,
  ensureWeChatEvents,
  forkThread,
  isThreadOpen,
  isThreadRunning,
  openSession,
  openNewSession,
  refreshWeChatState,
  refreshThreads,
  searchThreads,
  setToast,
  store,
  threadTitle,
  toastError,
  setThreadPinned,
} from "../composables/useCodex";
import { isThreadBound } from "../composables/useCodex/wechat";
import { useActionMenu, type ActionMenuItem } from "../composables/useActionMenu";
import { useSessionDialogs } from "../composables/useSessionDialogs";
import { formatRelativeTime } from "../lib/format";
import { groupSessions } from "../lib/sessionGroup";
import type { SessionGroup } from "../lib/sessionGroup";
import type { ThreadSummary } from "../lib/types";
import { debounce } from "../lib/debounce";
import {
  ICON_ARROW_DOWN,
  ICON_ARROW_RIGHT,
  ICON_DELETE,
  ICON_EXTERNAL_LINK,
  ICON_FOLDER_CLOSED,
  ICON_FOLDER_OPEN,
  ICON_FORK,
  ICON_OPEN,
  ICON_PIN,
  ICON_REFRESH,
  ICON_RENAME,
  ICON_SESSION,
  ICON_TERMINAL,
  ICON_WECHAT,
} from "../lib/icons";

// 重命名/删除确认弹窗状态
const confirmEl = ref<HTMLElement | null>(null);
/** 嵌套属性访问保留 Ref 对象（顶层绑定会被模板自动解包） */
const rootRefs = { confirmEl };
/** 正在操作微信接入的会话（null 表示弹窗关闭） */
const bindThread = ref<ThreadSummary | null>(null);
const {
  confirmDelete,
  editingId,
  editName,
  startRename,
  saveRename,
  cancelRename,
  askDelete,
  askDeleteGroup,
  cancelDelete,
  doDelete,
  confirmTitle,
  confirmMessage,
} = useSessionDialogs({ confirmEl });
const searchTerm = ref("");
/** 右键操作菜单：记录菜单项与位置 */
const {
  ctxMenu,
  openCtx,
  onWindowClick,
  onWindowScroll,
  onKeydown: onMenuKeydown,
} = useActionMenu({ width: 180, scrollScope: ".session-view" });
/** 默认收起；记录用户展开过的目录（启动恢复会话时经 pendingExpandGroupThread 展开对应分组） */
const expandedDirs = reactive(new Set<string>());
/** 启动恢复会话：消费 store.pendingExpandGroupThread（线程 id），展开其所属目录分组 */
watch(
  () => store.pendingExpandGroupThread,
  (id) => {
    if (!id) return;
    store.pendingExpandGroupThread = "";
    const hit = groupSessions(store.threads).find(
      (r): r is Extract<typeof r, { kind: "group" }> =>
        r.kind === "group" && r.group.threads.some((t) => t.id === id),
    );
    if (hit) expandedDirs.add(hit.group.key);
  },
  { immediate: true },
);
const debouncedSearch = debounce(() => void searchThreads(searchTerm.value), 300);

type RenderRow =
  | { kind: "folder"; group: SessionGroup; collapsed: boolean }
  | { kind: "item"; thread: ThreadSummary; inFolder: boolean };

function toggleDir(key: string) {
  if (expandedDirs.has(key)) expandedDirs.delete(key);
  else expandedDirs.add(key);
}

/** 目录行键盘操作：Enter/Space 与点击一致（Space 阻止默认滚动） */
function onFolderKeydown(key: string, e: KeyboardEvent) {
  if (e.key !== "Enter" && e.key !== " ") return;
  e.preventDefault();
  toggleDir(key);
}

/** 按目录分组后的渲染行：目录行 + （展开时）内部会话行 + 平铺会话行 */
const sessionRows = computed<RenderRow[]>(() => {
  const rows: RenderRow[] = [];
  for (const row of groupSessions(store.threads)) {
    if (row.kind === "group") {
      const collapsed = !expandedDirs.has(row.group.key);
      rows.push({ kind: "folder", group: row.group, collapsed });
      if (!collapsed) {
        for (const t of row.group.threads) {
          rows.push({ kind: "item", thread: t, inFolder: true });
        }
      }
    } else {
      rows.push({ kind: "item", thread: row.thread, inFolder: false });
    }
  }
  return rows;
});

function onSearchInput() {
  debouncedSearch.run();
}

function clearSearchInput() {
  searchTerm.value = "";
  debouncedSearch.cancel();
  clearSearch();
}

/** 刷新历史：搜索态重跑当前搜索，否则重新全量拉取 */
function onRefresh() {
  if (store.searchActive) {
    // 清掉未触发的防抖定时器，避免与手动刷新重复搜索
    debouncedSearch.cancel();
    void searchThreads(searchTerm.value);
  } else {
    void refreshThreads();
  }
}

/** 打开会话行右键菜单；重命名输入框内右键放行给全局编辑菜单 */
function openCtxMenu(t: ThreadSummary, e: MouseEvent) {
  if ((e.target as HTMLElement).closest?.(".rename-input")) return;
  const items: ActionMenuItem[] = [
    ...(isThreadOpen(t.id)
      ? []
      : [
          {
            label: "打开",
            icon: ICON_OPEN,
            action: () => void openSession(t.id),
          },
        ]),
    { label: "重命名", icon: ICON_RENAME, action: () => startRename(t) },
    {
      label: "分叉会话",
      icon: ICON_FORK,
      action: () => void forkThread(t.id),
    },
    {
      label: t.isPinned ? "取消固定" : "置顶固定",
      icon: ICON_PIN,
      action: () => void setThreadPinned(t.id, !t.isPinned),
    },
    {
      label: "微信接入",
      icon: ICON_WECHAT,
      nonzero: true,
      action: () => {
        bindThread.value = t;
        void refreshWeChatState();
      },
    },
    ...(isThreadOpen(t.id)
      ? []
      : [
          {
            label: "删除会话",
            icon: ICON_DELETE,
            danger: true,
            action: () => askDelete(t),
          },
        ]),
  ];
  openCtx(e, items);
}

/** 历史目录行右键菜单：新建会话（预置该分组目录）+ 在资源管理器中打开该目录 */
function openFolderCtxMenu(group: SessionGroup, e: MouseEvent) {
  const items: ActionMenuItem[] = [
    {
      label: "新建会话",
      icon: ICON_SESSION,
      action: () => void openNewSession(group.path),
    },
    {
      label: "在此打开终端",
      icon: ICON_TERMINAL,
      action: () => void openTerminalTab(group.path),
    },
    {
      label: "在资源管理器中打开",
      icon: ICON_EXTERNAL_LINK,
      action: () => revealInExplorer(group.path),
    },
  ];
  // 组内全部会话都已打开时隐藏「删除所有会话」
  if (group.threads.some((t) => !isThreadOpen(t.id))) {
    items.push({
      label: "删除所有会话",
      icon: ICON_DELETE,
      danger: true,
      action: () => askDeleteGroup(group),
    });
  }
  openCtx(e, items);
}

function revealInExplorer(path: string) {
  void invoke("reveal_path", { path }).catch((e) => {
    setToast(toastError(e));
  });
}

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  if (onMenuKeydown(e)) return;
  if (confirmDelete.value) cancelDelete();
}

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
  window.addEventListener("click", onWindowClick);
  window.addEventListener("scroll", onWindowScroll, true);
  // 面板常驻：启动即全量拉取历史
  void refreshThreads();
  // 微信绑定徽标与弹窗数据：启动即注册事件并拉取快照
  void ensureWeChatEvents();
  void refreshWeChatState();
});
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("click", onWindowClick);
  window.removeEventListener("scroll", onWindowScroll, true);
  debouncedSearch.cancel();
});
</script>

<template>
  <div class="session-view">
    <div class="panel-head">
      <div class="panel-search-group">
        <input
          v-model="searchTerm"
          class="panel-search"
          type="text"
          placeholder="搜索会话…"
          @input="onSearchInput()"
        />
        <button
          v-if="searchTerm"
          class="panel-search-clear"
          aria-label="清除搜索"
          v-tooltip="'清除搜索'"
          @click="clearSearchInput()"
        >
          ×
        </button>
        <button
          class="panel-refresh"
          aria-label="刷新"
          @click="onRefresh()"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_REFRESH" />
          </svg>
        </button>
      </div>
    </div>
    <div class="panel-list">
      <template
        v-for="row in sessionRows"
        :key="row.kind === 'folder' ? 'folder:' + row.group.key : row.thread.id"
      >
        <div
          v-if="row.kind === 'folder'"
          class="session-folder"
          :class="{ collapsed: row.collapsed }"
          role="button"
          tabindex="0"
          :aria-expanded="!row.collapsed"
          v-tooltip="row.group.path"
          @click="toggleDir(row.group.key)"
          @keydown="onFolderKeydown(row.group.key, $event)"
          @contextmenu="openFolderCtxMenu(row.group, $event)"
        >
          <svg class="folder-arrow" viewBox="0 0 24 24" aria-hidden="true">
            <path :d="row.collapsed ? ICON_ARROW_RIGHT : ICON_ARROW_DOWN" />
          </svg>
          <span class="folder-icon">
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path
                :d="row.collapsed ? ICON_FOLDER_CLOSED : ICON_FOLDER_OPEN"
              />
            </svg>
          </span>
          <span class="folder-name">{{ row.group.label }}</span>
          <span class="folder-count">{{ row.group.threads.length }}</span>
        </div>
        <div
          v-else
          class="session-item"
          :class="{
            active: row.thread.id === activeSessionTab()?.threadId,
            running: isThreadRunning(row.thread.id),
            'folder-item': row.inFolder,
          }"
          @click="openSession(row.thread.id)"
          @contextmenu="openCtxMenu(row.thread, $event)"
        >
          <span
            class="session-icon"
            :aria-hidden="isThreadBound(row.thread.id) ? undefined : 'true'"
            :aria-label="isThreadBound(row.thread.id) ? '已绑定微信' : undefined"
            v-tooltip="isThreadBound(row.thread.id) ? '已绑定微信' : undefined"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path
                :d="isThreadBound(row.thread.id) ? ICON_WECHAT : ICON_SESSION"
                fill="currentColor"
                stroke="none"
                :fill-rule="isThreadBound(row.thread.id) ? 'nonzero' : 'evenodd'"
              />
            </svg>
          </span>
          <span class="session-main">
            <input
              v-if="editingId === row.thread.id"
              v-model="editName"
              class="rename-input"
              @click.stop
              @keydown.enter="saveRename(row.thread)"
              @keydown.esc="cancelRename()"
              @blur="saveRename(row.thread)"
            />
            <template v-else>
              <span class="session-title-row">
                <span
                  v-if="row.thread.isPinned"
                  class="pin-badge"
                  aria-label="已置顶"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_PIN" />
                  </svg>
                </span>
                <span class="session-title">{{ threadTitle(row.thread) }}</span>
                <span
                  v-if="isThreadRunning(row.thread.id)"
                  class="session-run-dot"
                  v-tooltip="'后台运行中'"
                  aria-hidden="true"
                ></span>
              </span>
              <span
                v-if="store.searchActive && store.searchSnippets[row.thread.id]"
                class="session-snippet"
              >
                {{ store.searchSnippets[row.thread.id] }}
              </span>
          </template>
          </span>
          <span class="session-time">{{
            formatRelativeTime(row.thread.recencyAt ?? row.thread.updatedAt)
          }}</span>
        </div>
      </template>
      <div v-if="!store.threads.length && !store.loadingSessions" class="menu-note">
        暂无会话
      </div>
      <div v-if="store.loadingSessions && !store.threads.length" class="menu-note">
        加载中…
      </div>
    </div>
    <ContextMenu
      v-if="ctxMenu"
      :items="ctxMenu.items"
      :x="ctxMenu.x"
      :y="ctxMenu.y"
      @close="ctxMenu = null"
    />
    <ModalDialog
      v-if="confirmDelete"
      :title="confirmTitle"
      :root-ref="rootRefs.confirmEl"
    >
      {{ confirmMessage }}
      <template #foot>
        <button class="btn" @click="cancelDelete()">取消</button>
        <button class="btn danger" @click="doDelete()">删除</button>
      </template>
    </ModalDialog>
    <WechatBindDialog
      v-if="bindThread"
      :thread="bindThread"
      @close="bindThread = null"
    />
  </div>
</template>
