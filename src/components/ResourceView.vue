<script setup lang="ts">
import ContextMenu from "./ContextMenu.vue";
import ModalDialog from "./ModalDialog.vue";
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  setToast,
  workspace,
} from "../composables/useCodex";
import { revealGitFile } from "../composables/useGitChanges";
import { useActionMenu } from "../composables/useActionMenu";
import { useResourceDragDrop } from "../composables/useResourceDragDrop";
import { useResourceDialogs } from "../composables/useResourceDialogs";
import { useResourceMenus } from "../composables/useResourceMenus";
import {
  clearSearch,
  ensureEntryIcons,
  expanded,
  iconFor,
  onSearchInput,
  openTextEditor,
  openDocxEditor,
  probeTextEntry,
  refreshAll,
  revealActiveTab,
  revealInTree,
  rootEntry,
  rootError,
  runSearchNow,
  searchActive,
  searching,
  searchResults,
  searchTerm,
  selectedPath,
  setSessionFsActive,
  toggleDir,
  treeRows,
  loadingRoot,
  addAsAttachment,
  createTextFile,
  createFolder,
  loadDir,
  moveEntry,
  openImagePreview,
  openPdfPreview,
} from "../composables/useSessionFs";
import {
  formatFileSize,
  formatFileTime,
  type FsEntry,
  type ResourceRow,
} from "../lib/sessionFs";
import { previewTypeForName } from "../lib/preview";
import { isDocxPath } from "../lib/docx";
import {
  ICON_ARROW_DOWN,
  ICON_ARROW_RIGHT,
  ICON_AT,
  ICON_FILE,
  ICON_FOLDER_CLOSED,
  ICON_FOLDER_OPEN,
  ICON_REFRESH,
} from "../lib/icons";
import {
  activeTab,
  activeTabId,
} from "../composables/useEditorTabs";
import { TabKind } from "../lib/tabs";

const props = defineProps<{ active: boolean }>();

/** 是否正在显示活动会话标签：仅会话视图可见时提供「添加为会话附件」入口，
 *  避免附件进入隐藏/非活动会话的输入区（文件/diff/预览/终端标签激活时隐藏） */
const hasActiveSessionTab = computed(
  () =>
    activeTabId.value !== "" &&
    activeTab.value?.kind === TabKind.Chat,
);

const {
  ctxMenu,
  openCtx,
  onWindowClick,
  onWindowScroll,
  onKeydown: onMenuKeydown,
} = useActionMenu({ width: 190, scrollScope: ".resource-view" });
// 重命名/删除确认/属性弹窗状态
const {
  confirmDelete,
  propsEntry,
  propsLoading,
  editingPath,
  editName,
  startRename,
  saveRename,
  cancelRename,
  askDelete,
  cancelDelete,
  doDelete,
  openProps,
  formatAbsolute,
  deleteLabel,
} = useResourceDialogs();
// 右键菜单构建（根/目录/文件）
const {
  openRootMenu,
  openEntryMenu,
} = useResourceMenus({
  openCtx,
  rootEntry,
  hasActiveSessionTab,
  onCreateFolder,
  onCreateTextFile,
  requestOpen,
  startRename,
  askDelete,
  openProps,
});
// 树内拖拽移动（自绘指针拖拽）
/** 拖拽边界：资源文件区（.resource-list），指针离开时隐藏幽灵/清除高亮 */
const listRef = ref<HTMLElement | null>(null);
const {
  dragOverPath,
  dragGhost,
  suppressClick,
  isDragging,
  onRowPointerDown,
  cancelDrag,
} = useResourceDragDrop(moveEntry, listRef);

watch(
  () => props.active,
  (v) => {
    setSessionFsActive(v);
    // 面板切换进入时按活动标签（文件/diff/预览）工作区重载并定位
    if (v) void revealActiveTab();
  },
  { immediate: true },
);

// 可见行变化时懒加载缺失的文件系统图标（目录不在范围，仍用 SVG）
watch(treeRows, (rows) => {
  void ensureEntryIcons(rows.map((r) => r.entry));
});
watch(searchResults, (results) => {
  void ensureEntryIcons(results);
});

function entryIcon(entry: FsEntry): string {
  if (entry.isDir) {
    return expanded.has(entry.path) ? ICON_FOLDER_OPEN : ICON_FOLDER_CLOSED;
  }
  return ICON_FILE;
}

/** 文件系统图标（仅文件）；未缓存/取不到返回 undefined，渲染层回退 SVG */
function fileIcon(entry: FsEntry): string | undefined {
  return iconFor(entry) ?? undefined;
}

function fileMeta(entry: FsEntry): string {
  const parts = [formatFileSize(entry.size), formatFileTime(entry.modifiedAtMs)];
  return parts.filter(Boolean).join(" · ");
}

/** 打开前先按扩展名分发：PDF/图像 → 对应预览标签；.docx → 富文本编辑；其余探测内容：文本→编辑器；非文本→提示无法打开 */
async function requestOpen(entry: FsEntry) {
  const type = previewTypeForName(entry.name);
  if (type === "pdf") {
    openPdfPreview(entry);
    return;
  }
  if (type === "image") {
    openImagePreview(entry);
    return;
  }
  if (isDocxPath(entry.name)) {
    openDocxEditor(entry);
    return;
  }
  const ok = await probeTextEntry(entry);
  if (ok === null) return; // 探测失败：错误信息已 toast
  if (ok) {
    openTextEditor(entry);
    return;
  }
  setToast("该文件不是文本文件，无法打开");
}

/** 新建文件夹：创建成功后展开/加载父目录并自动进入行内重命名 */
async function onCreateFolder(parent: FsEntry) {
  const created = await createFolder(parent.path);
  if (!created) return;
  expanded.add(parent.path);
  await loadDir(parent.path, true);
  await nextTick();
  startRename(created);
}

/** 新建文本文件：创建成功后展开/加载父目录并自动进入行内重命名 */
async function onCreateTextFile(parent: FsEntry) {
  const created = await createTextFile(parent.path);
  if (!created) return;
  expanded.add(parent.path);
  await loadDir(parent.path, true);
  await nextTick();
  startRename(created);
}

function onRowContext(row: ResourceRow, e: MouseEvent) {
  if (row.kind === "root") openRootMenu(e);
  else openEntryMenu(row.entry, e);
}

/** 文件树行单击：文件 → 选中并在文本文件时打开预览；目录 → 折叠/展开 */
function onTreeRowClick(row: ResourceRow) {
  if (suppressClick.value) {
    suppressClick.value = false;
    return;
  }
  if (row.kind === "file") {
    selectedPath.value = row.entry.path;
    void requestOpen(row.entry);
    // 双向联动：资源面板点文件时同步高亮 Git 面板对应变更行（无变更时自然无匹配）
    revealGitFile(workspace.value, row.entry.path);
    return;
  }
  toggleDir(row.entry.path);
}

/** 搜索态结果单击：目录 → 定位回树；文件 → 探测后打开预览 */
function onSearchResultClick(entry: FsEntry) {
  if (entry.isDir) {
    void revealInTree(entry);
    return;
  }
  void requestOpen(entry);
  // 搜索态结果点击同样联动 Git 面板
  revealGitFile(workspace.value, entry.path);
}

function onRefresh() {
  if (searchActive.value) void runSearchNow();
  else void refreshAll();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  if (isDragging.value) {
    cancelDrag();
    return;
  }
  if (onMenuKeydown(e)) return;
  if (confirmDelete.value) cancelDelete();
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
            <path :d="ICON_REFRESH" />
          </svg>
        </button>
      </div>
    </div>

    <div ref="listRef" class="history-list resource-list">
      <!-- 搜索态：平铺结果 -->
      <template v-if="searchActive">
        <div
          v-for="entry in searchResults"
          :key="entry.path"
          class="resource-row resource-result"
          :class="{ active: selectedPath === entry.path }"
          :data-fs-path="entry.path"
          @click="onSearchResultClick(entry)"
          @contextmenu="openEntryMenu(entry, $event)"
        >
          <button
            v-if="hasActiveSessionTab"
            class="resource-add"
            :aria-label="`添加 ${entry.name} 为会话附件`"
            v-tooltip="'添加为会话附件'"
            @click.stop="addAsAttachment(entry)"
          ><svg viewBox="0 0 24 24" aria-hidden="true"><path :d="ICON_AT" /></svg></button>
          <span class="resource-icon">
            <img
              v-if="fileIcon(entry)"
              class="resource-icon-img"
              :src="fileIcon(entry)"
              alt=""
            />
            <svg v-else viewBox="0 0 24 24" aria-hidden="true">
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
            'resource-drop-target': dragOverPath === row.entry.path,
          }"
          :style="{ paddingLeft: 10 + row.depth * 14 + 'px' }"
          :data-fs-path="row.entry.path"
          v-tooltip="row.kind === 'root' ? row.entry.path : undefined"
          @click="onTreeRowClick(row)"
          @contextmenu="onRowContext(row, $event)"
          @pointerdown="row.kind !== 'root' && onRowPointerDown(row.entry, $event)"
        >
          <button
            v-if="hasActiveSessionTab && row.kind !== 'root'"
            class="resource-add"
            :aria-label="`添加 ${row.entry.name} 为会话附件`"
            v-tooltip="'添加为会话附件'"
            @click.stop="addAsAttachment(row.entry)"
          ><svg viewBox="0 0 24 24" aria-hidden="true"><path :d="ICON_AT" /></svg></button>
          <svg
            v-if="row.kind !== 'file'"
            class="resource-arrow"
            viewBox="0 0 24 24"
            aria-hidden="true"
          >
            <path :d="row.collapsed ? ICON_ARROW_RIGHT : ICON_ARROW_DOWN" />
          </svg>
          <span class="resource-icon">
            <img
              v-if="fileIcon(row.entry)"
              class="resource-icon-img"
              :src="fileIcon(row.entry)"
              alt=""
            />
            <svg v-else viewBox="0 0 24 24" aria-hidden="true">
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
      v-if="dragGhost"
      class="resource-drag-ghost"
      :style="{ left: dragGhost.x + 'px', top: dragGhost.y + 'px' }"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path :d="dragGhost.isDir ? ICON_FOLDER_CLOSED : ICON_FILE" />
      </svg>
      <span>{{ dragGhost.name }}</span>
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
      :title="`删除${confirmDelete.isDir ? '文件夹' : '文件'}`"
    >
      {{ deleteLabel }}
      <template #foot>
        <button class="btn" @click="cancelDelete()">取消</button>
        <button class="btn danger" @click="doDelete()">删除</button>
      </template>
    </ModalDialog>

    <ModalDialog
      v-if="propsEntry"
      :title="`属性 - ${propsEntry.name}`"
      closable
      body-class="resource-props"
      @close="propsEntry = null"
    >
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
      <template #foot>
        <button class="btn" @click="propsEntry = null">关闭</button>
      </template>
    </ModalDialog>
  </div>
</template>
