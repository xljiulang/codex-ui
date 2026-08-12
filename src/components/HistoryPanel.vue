<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import {
  clearSearch,
  deleteThread,
  openThread,
  renameThread,
  refreshThreads,
  searchThreads,
  store,
  threadTitle,
  togglePin,
} from "../composables/useCodex";
import { formatRelativeTime } from "../lib/format";
import { groupThreads } from "../lib/historyGroup";
import type { HistoryGroup } from "../lib/historyGroup";
import type { ThreadSummary } from "../lib/types";

/** 历史面板默认/最小宽度（px） */
const DEFAULT_PANEL_WIDTH = 264;
/** 文件夹行图标：收起=闭合文件夹，展开=打开文件夹 */
const FOLDER_CLOSED =
  "M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z";
const FOLDER_OPEN =
  "M20 6h-8l-2-2H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V8h16v10z";

const confirmThread = ref<ThreadSummary | null>(null);
const confirmEl = ref<HTMLElement | null>(null);
const searchTerm = ref("");
const editingId = ref<string | null>(null);
const editName = ref("");
/** 面板宽度：仅本次运行生效，不持久化 */
const panelWidth = ref(DEFAULT_PANEL_WIDTH);
/** 右键操作菜单：记录触发会话与菜单位置 */
const ctxMenu = ref<{ x: number; y: number; thread: ThreadSummary } | null>(
  null,
);
/** 默认收起；记录用户展开过的目录 */
const expandedDirs = reactive(new Set<string>());
let searchTimer: number | undefined;
let lastFocus: HTMLElement | null = null;
let resizeStartX = 0;
let resizeStartW = DEFAULT_PANEL_WIDTH;

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

/** 面板宽度钳制：最小为现有宽度 264px，最大为半个窗口宽度 */
function clampPanelWidth(w: number) {
  const max = Math.max(DEFAULT_PANEL_WIDTH, Math.floor(window.innerWidth / 2));
  return Math.min(Math.max(w, DEFAULT_PANEL_WIDTH), max);
}

function startResize(e: PointerEvent) {
  e.preventDefault();
  resizeStartX = e.clientX;
  resizeStartW = panelWidth.value;
  window.addEventListener("pointermove", onResizeMove);
  window.addEventListener("pointerup", onResizeUp);
}

function onResizeMove(e: PointerEvent) {
  // 面板在右侧：向左拖（X 减小）使面板变宽
  panelWidth.value = clampPanelWidth(resizeStartW + (resizeStartX - e.clientX));
}

function onResizeUp() {
  window.removeEventListener("pointermove", onResizeMove);
  window.removeEventListener("pointerup", onResizeUp);
}

function onWindowResize() {
  panelWidth.value = clampPanelWidth(panelWidth.value);
}

/** 打开会话行右键菜单；重命名输入框内右键放行给全局编辑菜单 */
function openCtxMenu(t: ThreadSummary, e: MouseEvent) {
  if ((e.target as HTMLElement).closest?.(".rename-input")) return;
  e.preventDefault();
  e.stopPropagation();
  const x = Math.min(e.clientX, window.innerWidth - 160);
  const y = Math.min(e.clientY, window.innerHeight - 4 * 30 - 12);
  ctxMenu.value = { x, y, thread: t };
}

function ctxOpen() {
  const t = ctxMenu.value?.thread;
  ctxMenu.value = null;
  if (t) void openThread(t.id);
}

function ctxTogglePin() {
  const t = ctxMenu.value?.thread;
  ctxMenu.value = null;
  if (t) void togglePin(t.id, !t.isPinned);
}

function ctxRename() {
  const t = ctxMenu.value?.thread;
  ctxMenu.value = null;
  if (t) startRename(t);
}

function ctxDelete() {
  const t = ctxMenu.value?.thread;
  ctxMenu.value = null;
  if (t) askDelete(t);
}

function onWindowClick() {
  ctxMenu.value = null;
}

function onWindowScroll() {
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
  window.addEventListener("resize", onWindowResize);
  // 面板常驻：启动即全量拉取历史
  void refreshThreads();
});
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("click", onWindowClick);
  window.removeEventListener("scroll", onWindowScroll, true);
  window.removeEventListener("resize", onWindowResize);
  onResizeUp();
});
</script>

<template>
  <aside class="history-panel" :style="{ width: panelWidth + 'px' }">
    <div
      class="history-resize-handle"
      aria-hidden="true"
      @pointerdown="startResize"
    ></div>
    <div class="history-head">
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
        >
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
      <button class="ctx-menu-item" @click="ctxOpen()">
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M14 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z"
          />
        </svg>
        <span>打开</span>
      </button>
      <button class="ctx-menu-item" @click="ctxTogglePin()">
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z"
          />
        </svg>
        <span>{{ ctxMenu.thread.isPinned ? "取消置顶" : "置顶" }}</span>
      </button>
      <button class="ctx-menu-item" @click="ctxRename()">
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"
          />
        </svg>
        <span>重命名</span>
      </button>
      <button class="ctx-menu-item danger" @click="ctxDelete()">
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"
          />
        </svg>
        <span>删除会话</span>
      </button>
    </div>
  </aside>

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
</template>
