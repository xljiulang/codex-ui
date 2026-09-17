
import { onBeforeUnmount, ref } from "vue";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { openPathInApp } from "./useSessionFs";
import { setToast, toastError } from "./useCodex";
import { findDropTarget, updateDropTarget } from "./dropTargets";

/**
 * 全局窗口级文件拖拽（唯一 Tauri onDragDropEvent 监听者）：
 * - 拖到 ComposerBar 输入区（dropTargets 命中）→ 派发给目标走附件逻辑，不打开文件；
 * - 拖到非输入区（AppHeader/资源树/空白等）→ openPathInApp 打开文件。
 * ComposerBar 不再自行监听 Tauri 事件，避免多监听者重复触发（多标签全收附件/双触发）。
 */
export function useGlobalDragDrop() {
  const dragging = ref(false);
  let dropUnlisten: UnlistenFn | undefined;

  async function setup() {
    try {
      dropUnlisten = await getCurrentWebview().onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === "enter" || p.type === "over") {
          dragging.value = true;
          // 物理像素坐标 → CSS 坐标（÷ devicePixelRatio），命中的输入区点亮高亮
          const hit = findDropTarget(logicalX(p), logicalY(p));
          updateDropTarget(hit);
        } else if (p.type === "leave") {
          dragging.value = false;
          updateDropTarget(null);
        } else if (p.type === "drop") {
          dragging.value = false;
          updateDropTarget(null);
          const paths = p.paths ?? [];
          if (!paths.length) return;
          const hit = findDropTarget(logicalX(p), logicalY(p));
          if (hit) {
            // 输入区命中：只走附件，不打开文件
            hit.onDropPaths(paths);
          } else {
            void handleDropPaths(paths);
          }
        }
      });
    } catch {
      // 非 Tauri 环境（浏览器/单测）忽略
    }
  }

  /** 物理像素坐标转 CSS 逻辑坐标（Tauri position 为物理像素） */
  function logicalX(p: { position?: { x: number } }): number {
    return (p.position?.x ?? 0) / (window.devicePixelRatio || 1);
  }

  function logicalY(p: { position?: { y: number } }): number {
    return (p.position?.y ?? 0) / (window.devicePixelRatio || 1);
  }

  async function handleDropPaths(paths: string[]) {
    for (const path of paths) {
      try {
        const ok = await openPathInApp(path);
        if (!ok) {
          setToast(`无法打开文件：${path}`);
        }
      } catch (e) {
        setToast(toastError(e));
      }
    }
  }

  onBeforeUnmount(() => {
    dropUnlisten?.();
  });

  return {
    dragging,
    setup,
  };
}
