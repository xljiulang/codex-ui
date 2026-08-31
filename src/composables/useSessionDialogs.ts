import { computed, nextTick, ref, type Ref } from "vue";
import {
  deleteThread,
  isThreadOpen,
  renameThread,
  setToast,
  threadTitle,
} from "./useCodex";
import type { SessionGroup } from "../lib/sessionGroup";
import type { ThreadSummary } from "../lib/types";

/** 删除确认目标：单条会话或整个目录分组 */
export type ConfirmDelete =
  | { kind: "thread"; thread: ThreadSummary }
  | { kind: "group"; group: SessionGroup };

/**
 * 会话列表行级弹窗/内联编辑状态：
 * 重命名（内联输入）、删除确认（单条/整组，聚焦管理并回焦）。
 */
export function useSessionDialogs(options: {
  /** 删除确认弹窗模板 ref（用于聚焦危险按钮） */
  confirmEl: Ref<HTMLElement | null>;
}) {
  const confirmDelete = ref<ConfirmDelete | null>(null);
  const editingId = ref<string | null>(null);
  const editName = ref("");
  let lastFocus: HTMLElement | null = null;

  function startRename(t: ThreadSummary) {
    editingId.value = t.id;
    editName.value = threadTitle(t);
    void nextTick(() => {
      document.querySelector<HTMLInputElement>(".rename-input")?.focus();
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

  function focusDangerButton() {
    void nextTick(() => {
      options.confirmEl.value
        ?.querySelector<HTMLElement>(".modal-foot .btn.danger")
        ?.focus();
    });
  }

  function askDelete(t: ThreadSummary) {
    confirmDelete.value = { kind: "thread", thread: t };
    lastFocus = document.activeElement as HTMLElement | null;
    focusDangerButton();
  }

  function askDeleteGroup(group: SessionGroup) {
    confirmDelete.value = { kind: "group", group };
    lastFocus = document.activeElement as HTMLElement | null;
    focusDangerButton();
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
      // 已打开标签的会话不可删除：仅删除未打开的，跳过的计数提示
      const closed = target.group.threads.filter((t) => !isThreadOpen(t.id));
      const skipped = target.group.threads.length - closed.length;
      await Promise.all(closed.map((t) => deleteThread(t.id)));
      if (skipped > 0) {
        setToast(`已跳过 ${skipped} 个已打开的会话`);
      }
    }
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
    const openCount = t.group.threads.filter((x) => isThreadOpen(x.id)).length;
    if (openCount === 0) {
      return `确定删除目录「${t.group.label}」下的所有会话（共 ${t.group.threads.length} 个）吗？此操作不可恢复。`;
    }
    const closedCount = t.group.threads.length - openCount;
    return `确定删除目录「${t.group.label}」下的 ${closedCount} 个未打开的会话吗？（${openCount} 个已打开将保留）此操作不可恢复。`;
  });

  return {
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
  };
}
