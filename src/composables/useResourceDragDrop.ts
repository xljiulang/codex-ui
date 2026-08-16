import { computed, onBeforeUnmount, onMounted, ref, type Ref } from "vue";
import type { FsEntry } from "../lib/sessionFs";

/** 指针拖拽移动超过该阈值才激活（区分普通点击） */
const DRAG_THRESHOLD = 5;

/**
 * 资源树内拖拽移动（自绘指针拖拽）：
 * 按下记录起点 → 移动超过阈值激活（显示浮动幽灵）→ 命中目录高亮 → 松开执行 moveEntry。
 */
export function useResourceDragDrop(
  moveEntry: (srcPath: string, destDir: string) => Promise<boolean>,
  /** 拖拽边界：资源文件区元素；缺省不限制（指针可越界，仅离区时隐藏幽灵/清除高亮） */
  listRef?: Ref<HTMLElement | null>,
) {
  const dragStart = ref<{ entry: FsEntry; x: number; y: number } | null>(null);
  const dragActive = ref(false);
  const dragOverPath = ref<string | null>(null);
  const dragGhost = ref<{
    x: number;
    y: number;
    name: string;
    isDir: boolean;
  } | null>(null);
  /** 拖拽结束后的合成 click 抑制标志（setTimeout(0) 复位） */
  const suppressClick = ref(false);

  /** 拖拽进行中（含按下未激活），供 Escape 取消判定 */
  const isDragging = computed(
    () => dragActive.value || dragStart.value !== null,
  );

  function onRowPointerDown(entry: FsEntry, e: PointerEvent) {
    if (e.button !== 0) return;
    const t = e.target as HTMLElement | null;
    if (t?.closest?.(".rename-input, .resource-add, .resource-arrow")) return;
    dragStart.value = { entry, x: e.clientX, y: e.clientY };
  }

  function cancelDrag() {
    dragStart.value = null;
    dragActive.value = false;
    dragOverPath.value = null;
    dragGhost.value = null;
    document.body.classList.remove("resource-dragging");
  }

  /** 拖拽目标合法性：源存在、目标不是源自身、目标不在源目录内（大小写不敏感） */
  function canDropTo(targetPath: string): boolean {
    const src = dragStart.value?.entry.path;
    if (!src) return false;
    const srcKey = src.replace(/\//g, "\\").toLowerCase();
    const targetKey = targetPath.replace(/\//g, "\\").toLowerCase();
    if (targetKey === srcKey) return false;
    return !targetKey.startsWith(srcKey + "\\");
  }

  /** 指针是否位于拖拽边界（资源文件区）矩形内；未提供边界时不限制 */
  function isInsideBoundary(x: number, y: number): boolean {
    const el = listRef?.value;
    if (!el) return true;
    const r = el.getBoundingClientRect();
    return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
  }

  function onWindowPointerMove(e: PointerEvent) {
    const start = dragStart.value;
    if (!start) return;
    if (!dragActive.value) {
      if (Math.hypot(e.clientX - start.x, e.clientY - start.y) < DRAG_THRESHOLD) {
        return;
      }
      dragActive.value = true;
      document.body.classList.add("resource-dragging");
    }
    // 离开资源文件区：隐藏幽灵并清除目标高亮（拖拽保持，回到区内恢复；区外松手不移动）
    if (!isInsideBoundary(e.clientX, e.clientY)) {
      dragGhost.value = null;
      dragOverPath.value = null;
      return;
    }
    dragGhost.value = {
      x: e.clientX,
      y: e.clientY,
      name: start.entry.name,
      isDir: start.entry.isDir,
    };
    // 命中检测：指针下最近的资源行；仅目录且合法时高亮
    const el = document.elementFromPoint(e.clientX, e.clientY);
    const row = el?.closest?.(".resource-row") as HTMLElement | null;
    const path = row?.dataset.fsPath;
    dragOverPath.value =
      path && row.classList.contains("resource-dir") && canDropTo(path)
        ? path
        : null;
  }

  function onWindowPointerUp() {
    const start = dragStart.value;
    dragStart.value = null;
    if (dragActive.value) {
      dragActive.value = false;
      suppressClick.value = true;
      window.setTimeout(() => {
        suppressClick.value = false;
      }, 0);
      const target = dragOverPath.value;
      dragOverPath.value = null;
      dragGhost.value = null;
      document.body.classList.remove("resource-dragging");
      if (start && target) {
        void moveEntry(start.entry.path, target);
      }
    }
  }

  onMounted(() => {
    window.addEventListener("pointermove", onWindowPointerMove);
    window.addEventListener("pointerup", onWindowPointerUp);
    window.addEventListener("pointercancel", cancelDrag);
  });
  onBeforeUnmount(() => {
    window.removeEventListener("pointermove", onWindowPointerMove);
    window.removeEventListener("pointerup", onWindowPointerUp);
    window.removeEventListener("pointercancel", cancelDrag);
  });

  return {
    dragActive,
    dragOverPath,
    dragGhost,
    suppressClick,
    isDragging,
    onRowPointerDown,
    cancelDrag,
  };
}
