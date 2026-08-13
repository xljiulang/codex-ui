<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  activateResourcesTab,
  clearSearch,
  deleteThread,
  newEmptyChat,
  openThread,
  renameThread,
  refreshThreads,
  searchThreads,
  store,
  threadTitle,
  toastError,
  togglePin,
} from "../composables/useCodex";
import { formatRelativeTime } from "../lib/format";
import { groupThreads } from "../lib/historyGroup";
import type { HistoryGroup } from "../lib/historyGroup";
import type { ThreadSummary } from "../lib/types";
import { focusComposer } from "../lib/composerFocus";

/** 文件夹行图标：收起=闭合文件夹，展开=打开文件夹 */
const FOLDER_CLOSED =
  "M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z";
const FOLDER_OPEN =
  "M20 6h-8l-2-2H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V8h16v10z";
/** 目录行折叠/展开箭头：收起=右箭头，展开=下箭头 */
const ICON_ARROW_RIGHT = "M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z";
const ICON_ARROW_DOWN =
  "M20 12l-1.41-1.41L13 16.17V4h-2v12.17l-5.58-5.59L4 12l8 8 8-8z";
const ICON_OPEN =
  "M19 19H5V5h7V3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z";
const ICON_RENAME =
  "M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z";
const ICON_PIN =
  "M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z";
const ICON_DELETE =
  "M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z";
const ICON_PLUS = "M11 5h2v6h6v2h-6v6h-2v-6H5v-2h6z";

interface CtxItem {
  label: string;
  icon: string;
  danger?: boolean;
  action: () => void;
}

const confirmThread = ref<ThreadSummary | null>(null);
const confirmEl = ref<HTMLElement | null>(null);
const searchTerm = ref("");
const editingId = ref<string | null>(null);
const editName = ref("");
/** 右键操作菜单：记录菜单项与位置 */
const ctxMenu = ref<{ x: number; y: number; items: CtxItem[] } | null>(null);
/** 默认收起；记录用户展开过的目录 */
const expandedDirs = reactive(new Set<string>());
let searchTimer: number | undefined;
let lastFocus: HTMLElement | null = null;

type RenderRow =
  | { kind: "folder"; group: HistoryGroup; collapsed: boolean }
  | { kind: "item"; thread: ThreadSummary; inFolder: boolean };

function toggleDir(key: string) {
  if (expandedDirs.has(key)) expandedDirs.delete(key);
  else expandedDirs.add(key);
}

/** 按目录分组后的渲染行：目录行 + （展开时）内部会话行 + 平铺会话行 */
const historyRows = computed<RenderRow[]>(() => {
  const rows: RenderRow[] = [];
  for (const row of groupThreads(store.threads)) {
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
  if (searchTimer) window.clearTimeout(searchTimer);
  searchTimer = window.setTimeout(() => {
    void searchThreads(searchTerm.value);
  }, 300);
}

function clearSearchInput() {
  searchTerm.value = "";
  if (searchTimer) window.clearTimeout(searchTimer);
  clearSearch();
}

/** 刷新历史：搜索态重跑当前搜索，否则重新全量拉取 */
function onRefresh() {
  if (store.searchActive) {
    void searchThreads(searchTerm.value);
  } else {
    void refreshThreads();
  }
}

function startRename(t: ThreadSummary) {
  editingId.value = t.id;
  editName.value = threadTitle(t);
  void nextTick(() => {
    (document.querySelector<HTMLInputElement>(".rename-input"))?.focus();
  });
}

function saveRename(t: ThreadSummary) {
  if (editingId.value !== t.id) return;
  editingId.value = null;
  if (editName.value.trim() && editName.value.trim() !== threadTitle(t)) {
    void renameThread(t.id, editName.value.trim());
  }
}

function cancelRename() {
  editingId.value = null;
}

function askDelete(t: ThreadSummary) {
  confirmThread.value = t;
  lastFocus = document.activeElement as HTMLElement | null;
  void nextTick(() => {
    confirmEl.value
      ?.querySelector<HTMLElement>(".modal-foot .btn.danger")
      ?.focus();
  });
}

function cancelDelete() {
  confirmThread.value = null;
  lastFocus?.focus?.();
  lastFocus = null;
}

async function doDelete() {
  const t = confirmThread.value;
  if (!t) return;
  confirmThread.value = null;
  lastFocus?.focus?.();
  lastFocus = null;
  await deleteThread(t.id);
}

function openCtx(e: MouseEvent, items: CtxItem[]) {
  e.preventDefault();
  e.stopPropagation();
  const x = Math.min(e.clientX, window.innerWidth - 180);
  const y = Math.min(e.clientY, window.innerHeight - items.length * 30 - 12);
  ctxMenu.value = { x, y, items };
}

/** 打开会话行右键菜单；重命名输入框内右键放行给全局编辑菜单 */
function openCtxMenu(t: ThreadSummary, e: MouseEvent) {
  if ((e.target as HTMLElement).closest?.(".rename-input")) return;
  openCtx(e, [
    { label: "打开", icon: ICON_OPEN, action: () => void openThread(t.id) },
    { label: "重命名", icon: ICON_RENAME, action: () => startRename(t) },
    {
      label: t.isPinned ? "取消固定" : "置顶固定",
      icon: ICON_PIN,
      action: () => void togglePin(t.id, !t.isPinned),
    },
    {
      label: "删除会话",
      icon: ICON_DELETE,
      danger: true,
      action: () => askDelete(t),
    },
  ]);
}

/** 历史目录行右键菜单：新建会话（预置该分组目录）+ 在资源管理器中打开该目录 */
function openFolderCtxMenu(group: HistoryGroup, e: MouseEvent) {
  openCtx(e, [
    {
      label: "新建会话",
      icon: ICON_PLUS,
      action: () => {
        store.showSettings = false;
        void newEmptyChat(group.path);
        // 与头部「新建会话」一致：进入新对话后聚焦输入框
        void nextTick(focusComposer);
        // 与头部「新建会话」一致：右侧面板切回资源管理器 Tab
        activateResourcesTab();
      },
    },
    {
      label: "在资源管理器中打开",
      icon: FOLDER_OPEN,
      action: () => revealInExplorer(group.path),
    },
  ]);
}

function revealInExplorer(path: string) {
  void invoke("reveal_path", { path }).catch((e) => {
    store.toast = toastError(e);
  });
}

function onWindowClick() {
  ctxMenu.value = null;
}

function onWindowScroll(e: Event) {
  // 仅面板自身滚动时关闭；聊天区等外部滚动（会话流式更新的吸底滚动）不影响菜单
  if (!(e.target instanceof HTMLElement)) return;
  if (!e.target.closest(".history-view")) return;
  ctxMenu.value = null;
}

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  if (ctxMenu.value) ctxMenu.value = null;
  else if (confirmThread.value) cancelDelete();
}

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
  window.addEventListener("click", onWindowClick);
  window.addEventListener("scroll", onWindowScroll, true);
  // 面板常驻：启动即全量拉取历史
  void refreshThreads();
});
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("click", onWindowClick);
  window.removeEventListener("scroll", onWindowScroll, true);
});
</script>

<template>
  <div class="history-view">
    <div class="history-head">
      <div class="history-search-group">
        <input
          v-model="searchTerm"
          class="history-search"
          type="text"
          placeholder="搜索会话…"
          @input="onSearchInput()"
        />
        <button
          v-if="searchTerm"
          class="history-search-clear"
          aria-label="清除搜索"
          v-tooltip="'清除搜索'"
          @click="clearSearchInput()"
        >
          ×
        </button>
        <button
          class="history-refresh"
          aria-label="刷新"
          v-tooltip="'刷新'"
          @click="onRefresh()"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M17.65 6.35A7.95 7.95 0 0 0 12 4a8 8 0 1 0 7.73 10h-2.08A6 6 0 1 1 12 6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z"
            />
          </svg>
        </button>
      </div>
    </div>
    <div class="history-list">
      <template
        v-for="row in historyRows"
        :key="row.kind === 'folder' ? 'folder:' + row.group.key : row.thread.id"
      >
        <div
          v-if="row.kind === 'folder'"
          class="history-folder"
          :class="{ collapsed: row.collapsed }"
          role="button"
          :aria-expanded="!row.collapsed"
          v-tooltip="row.group.path"
          @click="toggleDir(row.group.key)"
          @contextmenu="openFolderCtxMenu(row.group, $event)"
        >
          <svg class="folder-arrow" viewBox="0 0 24 24" aria-hidden="true">
            <path :d="row.collapsed ? ICON_ARROW_RIGHT : ICON_ARROW_DOWN" />
          </svg>
          <span class="folder-icon">
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="row.collapsed ? FOLDER_CLOSED : FOLDER_OPEN" />
            </svg>
          </span>
          <span class="folder-name">{{ row.group.label }}</span>
          <span class="folder-count">{{ row.group.threads.length }}</span>
        </div>
        <div
          v-else
          class="history-item"
          :class="{
            active: row.thread.id === store.currentThreadId,
            'folder-item': row.inFolder,
          }"
          @click="openThread(row.thread.id)"
          @contextmenu="openCtxMenu(row.thread, $event)"
        >
          <span class="history-main">
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
              <span class="history-title-row">
                <span
                  v-if="row.thread.isPinned"
                  class="pin-badge"
                  aria-label="已置顶"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path
                      d="M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z"
                    />
                  </svg>
                </span>
                <span class="history-title">{{ threadTitle(row.thread) }}</span>
              </span>
              <span
                v-if="store.searchActive && store.searchSnippets[row.thread.id]"
                class="history-snippet"
              >
                {{ store.searchSnippets[row.thread.id] }}
              </span>
          </template>
          </span>
          <span class="history-time">{{
            formatRelativeTime(row.thread.recencyAt ?? row.thread.updatedAt)
          }}</span>
        </div>
      </template>
      <div v-if="!store.threads.length && !store.loadingHistory" class="menu-note">
        暂无会话
      </div>
      <div v-if="store.loadingHistory && !store.threads.length" class="menu-note">
        加载中…
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
        :class="{ danger: it.danger }"
        @click="it.action(); ctxMenu = null"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path :d="it.icon" />
        </svg>
        <span>{{ it.label }}</span>
      </button>
    </div>
    <div v-if="confirmThread" class="modal-mask">
      <div ref="confirmEl" class="modal" tabindex="-1">
        <div class="modal-head">
          <span class="modal-title">删除会话</span>
        </div>
        <div class="modal-body">
          确定删除会话「{{ threadTitle(confirmThread) }}」吗？此操作不可恢复。
        </div>
        <div class="modal-foot">
          <button class="btn" @click="cancelDelete()">取消</button>
          <button class="btn danger" @click="doDelete()">删除</button>
        </div>
      </div>
    </div>
  </div>
</template>
