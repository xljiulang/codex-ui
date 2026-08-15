<script setup lang="ts">
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  reactive,
  ref,
  watch,
} from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  clearSearch,
  closeSessionTab,
  deleteThread,
  isThreadOpen,
  isThreadRunning,
  openHistorySession,
  openNewSession,
  renameThread,
  refreshThreads,
  searchThreads,
  setToast,
  store,
  threadTitle,
  toastError,
  togglePin,
} from "../composables/useCodex";
import { useActionMenu, type CtxItem } from "../composables/useActionMenu";
import { formatRelativeTime } from "../lib/format";
import { groupThreads } from "../lib/historyGroup";
import type { HistoryGroup } from "../lib/historyGroup";
import type { ThreadSummary } from "../lib/types";
import { debounce } from "../lib/debounce";
import {
  ICON_ARROW_DOWN,
  ICON_ARROW_RIGHT,
  ICON_CLOSE_ALL,
  ICON_DELETE,
  ICON_FOLDER_CLOSED,
  ICON_FOLDER_OPEN,
  ICON_OPEN,
  ICON_PIN,
  ICON_PLUS,
  ICON_REFRESH,
  ICON_RENAME,
} from "../lib/icons";

/** 删除确认目标：单条会话或整个目录分组 */
type ConfirmDelete =
  | { kind: "thread"; thread: ThreadSummary }
  | { kind: "group"; group: HistoryGroup };
const confirmDelete = ref<ConfirmDelete | null>(null);
const confirmEl = ref<HTMLElement | null>(null);
const searchTerm = ref("");
const editingId = ref<string | null>(null);
const editName = ref("");
/** 右键操作菜单：记录菜单项与位置 */
const {
  ctxMenu,
  openCtx,
  onWindowClick,
  onWindowScroll,
  onKeydown: onMenuKeydown,
} = useActionMenu({ width: 180, scrollScope: ".history-view" });
/** 默认收起；记录用户展开过的目录 */
const expandedDirs = reactive(new Set<string>());
/** 首个目录仅首次载入时自动展开一次；之后完全由用户控制（含刷新后不重置） */
let firstFolderAutoExpanded = false;
watch(
  () => store.threads,
  () => {
    if (firstFolderAutoExpanded) return;
    const first = groupThreads(store.threads).find(
      (r): r is Extract<typeof r, { kind: "group" }> => r.kind === "group",
    );
    if (!first) return;
    expandedDirs.add(first.group.key);
    firstFolderAutoExpanded = true;
  },
  { immediate: true },
);
const debouncedSearch = debounce(() => void searchThreads(searchTerm.value), 300);
let lastFocus: HTMLElement | null = null;

type RenderRow =
  | { kind: "folder"; group: HistoryGroup; collapsed: boolean }
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
  confirmDelete.value = { kind: "thread", thread: t };
  lastFocus = document.activeElement as HTMLElement | null;
  void nextTick(() => {
    confirmEl.value
      ?.querySelector<HTMLElement>(".modal-foot .btn.danger")
      ?.focus();
  });
}

function askDeleteGroup(group: HistoryGroup) {
  confirmDelete.value = { kind: "group", group };
  lastFocus = document.activeElement as HTMLElement | null;
  void nextTick(() => {
    confirmEl.value
      ?.querySelector<HTMLElement>(".modal-foot .btn.danger")
      ?.focus();
  });
}

function cancelDelete() {
  confirmDelete.value = null;
  lastFocus?.focus?.();
  lastFocus = null;
}

async function doDelete() {
  const target = confirmDelete.value;
  if (!target) return;
  confirmDelete.value = null;
  lastFocus?.focus?.();
  lastFocus = null;
  if (target.kind === "thread") {
    await deleteThread(target.thread.id);
  } else {
    await Promise.all(target.group.threads.map((t) => deleteThread(t.id)));
  }
}

/** 打开会话行右键菜单；重命名输入框内右键放行给全局编辑菜单 */
function openCtxMenu(t: ThreadSummary, e: MouseEvent) {
  if ((e.target as HTMLElement).closest?.(".rename-input")) return;
  const items: CtxItem[] = [
    {
      label: "打开",
      icon: ICON_OPEN,
      action: () => void openHistorySession(t.id),
    },
    ...(isThreadOpen(t.id)
      ? [
          {
            label: "关闭标签",
            icon: ICON_CLOSE_ALL,
            action: () => void closeSessionTab(findSessionTabId(t.id)),
          },
        ]
      : []),
    { label: "重命名", icon: ICON_RENAME, action: () => startRename(t) },
    {
      label: t.isPinned ? "取消固定" : "置顶固定",
      icon: ICON_PIN,
      action: () => void togglePin(t.id, !t.isPinned),
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

/** 会话标签 id（已打开时才调用） */
function findSessionTabId(threadId: string): string {
  const tab = store.sessionTabs.find((t) => t.threadId === threadId);
  return tab?.id ?? "";
}

/** 历史目录行右键菜单：新建会话（预置该分组目录）+ 在资源管理器中打开该目录 */
function openFolderCtxMenu(group: HistoryGroup, e: MouseEvent) {
  openCtx(e, [
    {
      label: "新建会话",
      icon: ICON_PLUS,
      action: () => void openNewSession(group.path),
    },
    {
      label: "在资源管理器中打开",
      icon: ICON_FOLDER_OPEN,
      action: () => revealInExplorer(group.path),
    },
    {
      label: "删除所有会话",
      icon: ICON_DELETE,
      danger: true,
      action: () => askDeleteGroup(group),
    },
  ]);
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

const confirmTitle = computed(() =>
  confirmDelete.value?.kind === "group" ? "删除所有会话" : "删除会话",
);
const confirmMessage = computed(() => {
  const t = confirmDelete.value;
  if (!t) return "";
  if (t.kind === "thread") {
    return `确定删除会话「${threadTitle(t.thread)}」吗？此操作不可恢复。`;
  }
  return `确定删除目录「${t.group.label}」下的所有会话（共 ${t.group.threads.length} 个）吗？此操作不可恢复。`;
});

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
  debouncedSearch.cancel();
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
            <path :d="ICON_REFRESH" />
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
          class="history-item"
          :class="{
            active: row.thread.id === store.currentThreadId,
            open: isThreadOpen(row.thread.id),
            running: isThreadRunning(row.thread.id),
            'folder-item': row.inFolder,
          }"
          @click="openHistorySession(row.thread.id)"
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
                    <path :d="ICON_PIN" />
                  </svg>
                </span>
                <span class="history-title">{{ threadTitle(row.thread) }}</span>
                <span
                  v-if="isThreadOpen(row.thread.id)"
                  class="history-open-badge"
                  title="已在会话标签中打开"
                >
                  已打开
                </span>
                <span
                  v-if="isThreadRunning(row.thread.id)"
                  class="history-run-dot"
                  title="后台运行中"
                  aria-hidden="true"
                ></span>
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
    <div v-if="confirmDelete" class="modal-mask">
      <div ref="confirmEl" class="modal" tabindex="-1">
        <div class="modal-head">
          <span class="modal-title">{{ confirmTitle }}</span>
        </div>
        <div class="modal-body">
          {{ confirmMessage }}
        </div>
        <div class="modal-foot">
          <button class="btn" @click="cancelDelete()">取消</button>
          <button class="btn danger" @click="doDelete()">删除</button>
        </div>
      </div>
    </div>
  </div>
</template>
