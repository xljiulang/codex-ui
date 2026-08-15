import { computed, nextTick, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { deleteEntry, renameEntry } from "./useSessionFs";
import { setToast, toastError, workspace } from "./useCodex";
import type { FsEntry } from "../lib/sessionFs";

/**
 * 资源树行级弹窗/内联编辑状态：
 * 重命名（内联输入）、删除确认、属性弹窗（拉取元信息）。
 */
export function useResourceDialogs() {
  const confirmDelete = ref<FsEntry | null>(null);
  const propsEntry = ref<FsEntry | null>(null);
  const propsLoading = ref(false);
  const editingPath = ref<string | null>(null);
  const editName = ref("");

  function startRename(entry: FsEntry) {
    editingPath.value = entry.path;
    editName.value = entry.name;
    void nextTick(() => {
      document
        .querySelector<HTMLInputElement>(".resource-rename-input")
        ?.focus();
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
        workspace: workspace.value,
        path: entry.path,
      });
      propsEntry.value = meta;
    } catch (e) {
      setToast(toastError(e));
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

  const deleteLabel = computed(() => {
    const e = confirmDelete.value;
    if (!e) return "";
    return e.isDir
      ? `确定删除文件夹「${e.name}」吗？其内容将一并删除，此操作不可恢复。`
      : `确定删除文件「${e.name}」吗？此操作不可恢复。`;
  });

  return {
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
  };
}
