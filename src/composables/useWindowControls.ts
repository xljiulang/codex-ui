// 自绘标题栏窗口控制：最小化 / 最大化(还原) / 关闭，以及标题栏拖动窗口、双击最大化/还原。
// 非 Tauri 环境（浏览器预览/单测）静默降级为无操作，风格对齐 windowTitle.ts。
import { onBeforeUnmount, onMounted, ref, type Ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";

export interface WindowControls {
  /** 当前是否处于最大化（控制最大化/还原图标展示） */
  isMaximized: Ref<boolean>;
  /** 最小化窗口 */
  minimize: () => void;
  /** 切换最大化/还原 */
  toggleMaximize: () => Promise<void>;
  /** 关闭（复用现有「关闭→隐藏到系统托盘」逻辑） */
  close: () => void;
  /** 标题栏左键按下：非交互元素时开始拖动窗口 */
  onTitlebarMouseDown: (event: MouseEvent) => void;
}

/** 标题栏交互元素（按钮/输入等）不触发拖动与双击最大化 */
const NON_DRAG_SELECTOR = "button, a, input, textarea, [data-no-drag]";

/**
 * 窗口控制逻辑（供 AppHeader 自绘标题栏使用）：
 * - isMaximized 在挂载时初始化，并按窗口 resize（含 Win+↑/↓、拖到顶部等系统最大化）同步；
 * - 拖动/双击仅在非交互元素上生效，点击窗口控制按钮不会误触；
 * - 双击最大化用 mousedown 的 event.detail===2 判定，与 Tauri 内置 data-tauri-drag-region
 *   一致，避免独立 dblclick 事件被拖动打断。
 */
export function useWindowControls(): WindowControls {
  const isMaximized = ref(false);
  let unlistenResized: UnlistenFn | undefined;

  async function refreshMaximized(): Promise<void> {
    try {
      isMaximized.value = await getCurrentWindow().isMaximized();
    } catch {
      // 非 Tauri 环境（浏览器预览/单测）忽略
    }
  }

  async function toggleMaximize(): Promise<void> {
    try {
      const win = getCurrentWindow();
      await win.toggleMaximize();
      // 切换后立即读取真实状态（部分平台原生按钮/系统快捷键触发的切换只能靠 isMaximized 对账）
      isMaximized.value = await win.isMaximized();
    } catch {
      // 非 Tauri 环境忽略
    }
  }

  function minimize(): void {
    try {
      void getCurrentWindow().minimize();
    } catch {
      // 非 Tauri 环境忽略
    }
  }

  function close(): void {
    try {
      void getCurrentWindow().close();
    } catch {
      // 非 Tauri 环境忽略
    }
  }

  function onTitlebarMouseDown(event: MouseEvent): void {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest(NON_DRAG_SELECTOR)) return;
    try {
      if (event.detail === 2) {
        // 双击标题栏空白区：最大化/还原（不进入拖动，避免与拖动冲突）
        void toggleMaximize();
      } else {
        void getCurrentWindow().startDragging();
      }
    } catch {
      // 非 Tauri 环境忽略
    }
  }

  onMounted(() => {
    void refreshMaximized();
    try {
      const win = getCurrentWindow();
      void win
        .onResized(() => {
          void refreshMaximized();
        })
        .then((fn) => {
          unlistenResized = fn;
        });
    } catch {
      // 非 Tauri 环境忽略
    }
  });

  onBeforeUnmount(() => {
    unlistenResized?.();
  });

  return {
    isMaximized,
    minimize,
    toggleMaximize,
    close,
    onTitlebarMouseDown,
  };
}
