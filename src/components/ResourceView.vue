<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { store, toastError } from "../composables/useCodex";
import {
  clearSearch,
  copyEntry,
  deleteEntry,
  expanded,
  onSearchInput,
  openTextPreview,
  pasteInto,
  refreshAll,
  renameEntry,
  revealInExplorer,
  revealInTree,
  rootEntry,
  rootError,
  runSearchNow,
  searchActive,
  searching,
  searchResults,
  searchTerm,
  selectedPath,
  sessionRoot,
  setSessionFsActive,
  toggleDir,
  treeRows,
  loadingRoot,
  addAsAttachment,
} from "../composables/useSessionFs";
import {
  formatFileSize,
  formatFileTime,
  isTextFile,
  type FsEntry,
  type ResourceRow,
} from "../lib/sessionFs";

const props = defineProps<{ active: boolean }>();

const ICON_FOLDER_CLOSED =
  "M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z";
const ICON_FOLDER_OPEN =
  "M20 6h-8l-2-2H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V8h16v10z";
const ICON_FILE =
  "M14 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z";
const ICON_PASTE =
  "M19 3h-4.18C14.4 1.84 13.3 1 12 1c-1.3 0-2.4.84-2.82 2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm-7 0c.55 0 1 .45 1 1s-.45 1-1 1-1-.45-1-1 .45-1 1-1zm2 14H7v-2h7v2zm3-4H7v-2h10v2zm0-4H7V7h10v2z";
const ICON_COPY =
  "M16 1H4c-1.1 0-2 .9-2 2v14h2V3h12V1zm3 4H8c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h11c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm0 16H8V7h11v14z";
const ICON_DELETE =
  "M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z";
const ICON_RENAME =
  "M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z";
const ICON_REVEAL =
  "M20 6h-8l-2-2H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V8h16v10z";
const ICON_ATTACH =
  "M16.5 6v11.5c0 2.21-1.79 4-4 4s-4-1.79-4-4V5a2.5 2.5 0 0 1 5 0v10.5c0 .55-.45 1-1 1s-1-.45-1-1V6H10v9.5a2.5 2.5 0 0 0 5 0V5c0-1.93-1.57-3.5-3.5-3.5S8 3.07 8 5v12.5c0 2.76 2.24 5 5 5s5-2.24 5-5V6h-1.5z";
const ICON_INFO =
  "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-6h2v6zm0-8h-2V7h2v2z";
const ICON_OPEN =
  "M19 19H5V5h7V3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z";

interface CtxItem {
  label: string;
  icon: string;
  danger?: boolean;
  action: () => void;
}

const ctxMenu = ref<{ x: number; y: number; items: CtxItem[] } | null>(null);
const confirmDelete = ref<FsEntry | null>(null);
const propsEntry = ref<FsEntry | null>(null);
const propsLoading = ref(false);
const editingPath = ref<string | null>(null);
const editName = ref("");

watch(
  () => props.active,
  (v) => setSessionFsActive(v),
  { immediate: true },
);

function entryIcon(entry: FsEntry): string {
  if (entry.isDir) {
    return expanded.has(entry.path) ? ICON_FOLDER_OPEN : ICON_FOLDER_CLOSED;
  }
  return ICON_FILE;
}

function fileMeta(entry: FsEntry): string {
  const parts = [formatFileSize(entry.size), formatFileTime(entry.modifiedAtMs)];
  return parts.filter(Boolean).join(" · ");
}

function openCtx(e: MouseEvent, items: CtxItem[]) {
  e.preventDefault();
  e.stopPropagation();
  const x = Math.min(e.clientX, window.innerWidth - 190);
  const y = Math.min(e.clientY, window.innerHeight - items.length * 30 - 12);
  ctxMenu.value = { x, y, items };
}

function openRootMenu(e: MouseEvent) {
  const root = rootEntry.value;
  if (!root) return;
  openCtx(e, [
    {
      label: "粘贴",
      icon: ICON_PASTE,
      action: () => void pasteInto(root.path),
    },
    {
      label: "在资源管理器中打开",
      icon: ICON_REVEAL,
      action: () => revealInExplorer(root.path),
    },
  ]);
}

function openDirMenu(entry: FsEntry, e: MouseEvent) {
  openCtx(e, [
    { label: "复制", icon: ICON_COPY, action: () => copyEntry(entry) },
    {
      label: "粘贴",
      icon: ICON_PASTE,
      action: () => void pasteInto(entry.path),
    },
    {
      label: "删除",
      icon: ICON_DELETE,
      danger: true,
      action: () => askDelete(entry),
    },
    {
      label: "重命名",
      icon: ICON_RENAME,
      action: () => startRename(entry),
    },
    {
      label: "添加为会话附件",
      icon: ICON_ATTACH,
      action: () => addAsAttachment(entry),
    },
    {
      label: "在资源管理器中打开",
      icon: ICON_REVEAL,
      action: () => revealInExplorer(entry.path),
    },
  ]);
}

function openFileMenu(entry: FsEntry, e: MouseEvent) {
  const items: CtxItem[] = [];
  if (isTextFile(entry.name)) {
    items.push({
      label: "打开",
      icon: ICON_OPEN,
      action: () => openTextPreview(entry),
    });
  }
  items.push(
    { label: "复制", icon: ICON_COPY, action: () => copyEntry(entry) },
    {
      label: "属性",
      icon: ICON_INFO,
      action: () => void openProps(entry),
    },
    {
      label: "删除",
      icon: ICON_DELETE,
      danger: true,
      action: () => askDelete(entry),
    },
    {
      label: "重命名",
      icon: ICON_RENAME,
      action: () => startRename(entry),
    },
    {
      label: "添加为会话附件",
      icon: ICON_ATTACH,
      action: () => addAsAttachment(entry),
    },
    {
      label: "在资源管理器中打开",
      icon: ICON_REVEAL,
      action: () => revealInExplorer(entry.path),
    },
  );
  openCtx(e, items);
}

function openEntryMenu(entry: FsEntry, e: MouseEvent) {
  if (entry.isDir) openDirMenu(entry, e);
  else openFileMenu(entry, e);
}

function onRowContext(row: ResourceRow, e: MouseEvent) {
  if (row.kind === "root") openRootMenu(e);
  else openEntryMenu(row.entry, e);
}

function startRename(entry: FsEntry) {
  editingPath.value = entry.path;
  editName.value = entry.name;
  void nextTick(() => {
    (document.querySelector<HTMLInputElement>(".resource-rename-input"))?.focus();
  });
}

function saveRename(entry: FsEntry) {
  if (editingPath.value !== entry.path) return;
  editingPath.value = null;
  const name = editName.value.trim();
  if (name && name !== entry.name) {
    void renameEntry(entry.path, name);
  }
}

function cancelRename() {
  editingPath.value = null;
}

function askDelete(entry: FsEntry) {
  confirmDelete.value = entry;
}

function cancelDelete() {
  confirmDelete.value = null;
}

function doDelete() {
  const entry = confirmDelete.value;
  if (!entry) return;
  confirmDelete.value = null;
  void deleteEntry(entry.path);
}

async function openProps(entry: FsEntry) {
  propsEntry.value = entry;
  propsLoading.value = true;
  try {
    const meta = await invoke<FsEntry>("session_fs_metadata", {
      root: sessionRoot.value,
      path: entry.path,
    });
    propsEntry.value = meta;
  } catch (e) {
    store.toast = toastError(e);
  } finally {
    propsLoading.value = false;
  }
}

function formatAbsolute(ms: number): string {
  if (!ms) return "-";
  return new Date(ms).toLocaleString("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function onRefresh() {
  if (searchActive.value) void runSearchNow();
  else void refreshAll();
}

function onWindowClick() {
  ctxMenu.value = null;
}

function onWindowScroll(e: Event) {
  // 仅面板自身滚动时关闭；聊天区等外部滚动（会话流式更新的吸底滚动）不影响菜单
  if (!(e.target instanceof HTMLElement)) return;
  if (!e.target.closest(".resource-view")) return;
  ctxMenu.value = null;
}

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  if (ctxMenu.value) ctxMenu.value = null;
  else if (confirmDelete.value) cancelDelete();
  else if (propsEntry.value) propsEntry.value = null;
  else if (editingPath.value) cancelRename();
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

const deleteLabel = computed(() => {
  const e = confirmDelete.value;
  if (!e) return "";
  return e.isDir
    ? `确定删除文件夹「${e.name}」吗？其内容将一并删除，此操作不可恢复。`
    : `确定删除文件「${e.name}」吗？此操作不可恢复。`;
});
</script>

<template>
  <div class="resource-view">
    <div class="history-head">
      <div class="history-search-group">
        <input
          v-model="searchTerm"
          class="history-search"
          type="text"
          placeholder="搜索资源…"
          @input="onSearchInput()"
        />
        <button
          v-if="searchTerm"
          class="history-search-clear"
          aria-label="清除搜索"
          v-tooltip="'清除搜索'"
          @click="clearSearch()"
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

    <div class="history-list resource-list">
      <!-- 搜索态：平铺结果 -->
      <template v-if="searchActive">
        <div
          v-for="entry in searchResults"
          :key="entry.path"
          class="resource-row resource-result"
          :class="{ active: selectedPath === entry.path }"
          :data-fs-path="entry.path"
          @click="revealInTree(entry)"
          @contextmenu="openEntryMenu(entry, $event)"
        >
          <span class="resource-icon">
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="entryIcon(entry)" />
            </svg>
          </span>
          <span class="resource-main">
            <span class="resource-name">{{ entry.name }}</span>
            <span class="resource-relpath">{{ entry.relPath }}</span>
          </span>
        </div>
        <div v-if="!searching && !searchResults.length" class="menu-note">
          无匹配结果
        </div>
        <div v-if="searching" class="menu-note">搜索中…</div>
      </template>

      <!-- 普通态：文件树 -->
      <template v-else>
        <div
          v-for="row in treeRows"
          :key="row.kind === 'root' ? 'root' : row.entry.path"
          class="resource-row"
          :class="{
            'resource-root': row.kind === 'root',
            'resource-dir': row.kind === 'dir',
            'resource-file': row.kind === 'file',
            collapsed: row.kind !== 'file' && row.collapsed,
            active: row.kind !== 'root' && selectedPath === row.entry.path,
          }"
          :style="{ paddingLeft: 10 + row.depth * 14 + 'px' }"
          :data-fs-path="row.entry.path"
          @click="
            row.kind === 'file'
              ? (selectedPath = row.entry.path)
              : toggleDir(row.entry.path)
          "
          @contextmenu="onRowContext(row, $event)"
        >
          <span class="resource-icon">
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="entryIcon(row.entry)" />
            </svg>
          </span>
          <span class="resource-main">
            <input
              v-if="editingPath === row.entry.path"
              v-model="editName"
              class="rename-input resource-rename-input"
              @click.stop
              @keydown.enter="saveRename(row.entry)"
              @keydown.esc="cancelRename()"
              @blur="saveRename(row.entry)"
            />
            <template v-else>
              <span class="resource-name">{{ row.entry.name }}</span>
            </template>
          </span>
          <span class="resource-side">
            <span
              v-if="row.kind !== 'file'"
              class="resource-time"
            >{{ formatFileTime(row.entry.modifiedAtMs) }}</span>
            <span
              v-if="row.kind !== 'file'"
              class="resource-count"
            >{{ row.entry.childCount ?? 0 }}</span>
            <span
              v-else
              class="resource-time"
            >{{ fileMeta(row.entry) }}</span>
          </span>
        </div>
        <div v-if="loadingRoot && !rootEntry" class="menu-note">加载中…</div>
        <div v-if="rootError && !rootEntry" class="menu-note resource-error">
          {{ rootError }}
        </div>
        <div v-if="!rootError && !rootEntry && !loadingRoot" class="menu-note">
          暂无工作目录
        </div>
      </template>
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
      <div class="modal" tabindex="-1">
        <div class="modal-head">
          <span class="modal-title">删除{{ confirmDelete.isDir ? "文件夹" : "文件" }}</span>
        </div>
        <div class="modal-body">{{ deleteLabel }}</div>
        <div class="modal-foot">
          <button class="btn" @click="cancelDelete()">取消</button>
          <button class="btn danger" @click="doDelete()">删除</button>
        </div>
      </div>
    </div>

    <div v-if="propsEntry" class="modal-mask">
      <div class="modal" tabindex="-1">
        <div class="modal-head">
          <span class="modal-title">属性 - {{ propsEntry.name }}</span>
          <button class="modal-close" aria-label="关闭" @click="propsEntry = null">
            ×
          </button>
        </div>
        <div class="modal-body resource-props">
          <div class="resource-prop">
            <span class="resource-prop-key">名称</span>
            <span class="resource-prop-value">{{ propsEntry.name }}</span>
          </div>
          <div class="resource-prop">
            <span class="resource-prop-key">类型</span>
            <span class="resource-prop-value">
              {{ propsEntry.isDir ? "文件夹" : "文件" }}
            </span>
          </div>
          <div class="resource-prop">
            <span class="resource-prop-key">完整路径</span>
            <span class="resource-prop-value">{{ propsEntry.path }}</span>
          </div>
          <div class="resource-prop">
            <span class="resource-prop-key">大小</span>
            <span class="resource-prop-value">
              {{ propsEntry.isDir
                ? (propsEntry.childCount ?? 0) + " 项"
                : formatFileSize(propsEntry.size) || "-" }}
            </span>
          </div>
          <div class="resource-prop">
            <span class="resource-prop-key">修改时间</span>
            <span class="resource-prop-value">
              {{ formatAbsolute(propsEntry.modifiedAtMs) }}
            </span>
          </div>
          <div class="resource-prop">
            <span class="resource-prop-key">创建时间</span>
            <span class="resource-prop-value">
              {{ formatAbsolute(propsEntry.createdAtMs) }}
            </span>
          </div>
          <div v-if="propsLoading" class="menu-note">读取中…</div>
        </div>
        <div class="modal-foot">
          <button class="btn" @click="propsEntry = null">关闭</button>
        </div>
      </div>
    </div>
  </div>
</template>
