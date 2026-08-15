import { ref } from "vue";
import { clampMenuPos } from "../lib/ctxMenu";

export interface CtxItem {
  label: string;
  icon?: string;
  /** 位图图标（如系统文件图标 PNG data URI）；存在时优先于 icon 渲染 */
  img?: string;
  danger?: boolean;
  action: () => void;
}

export interface ActionMenuState {
  x: number;
  y: number;
  items: CtxItem[];
}

/**
 * 操作型右键菜单脚手架：管理菜单状态、定位与全局关闭监听。
 * 组件负责模板渲染与各自的额外 Escape/点击级联；
 * 全局复制/链接菜单（useContextMenu）职责不同，不共用本实现。
 */
export function useActionMenu(options?: {
  width?: number;
  scrollScope?: string;
}) {
  const width = options?.width ?? 180;
  const scrollScope = options?.scrollScope ?? "";
  const ctxMenu = ref<ActionMenuState | null>(null);

  function openCtx(e: MouseEvent, items: CtxItem[]) {
    e.preventDefault();
    e.stopPropagation();
    const pos = clampMenuPos(
      e.clientX,
      e.clientY,
      width,
      items.length * 30 + 12,
    );
    ctxMenu.value = { x: pos.x, y: pos.y, items };
  }

  function closeCtx() {
    ctxMenu.value = null;
  }

  function onWindowClick() {
    ctxMenu.value = null;
  }

  function onWindowScroll(e: Event) {
    if (!scrollScope) return;
    if (!(e.target instanceof Element)) return;
    if (!e.target.closest(scrollScope)) return;
    ctxMenu.value = null;
  }

  /** Escape：菜单开着则关闭并返回 true，供调用方级联其它 Escape 行为 */
  function onKeydown(e: KeyboardEvent): boolean {
    if (e.key !== "Escape") return false;
    if (ctxMenu.value) {
      ctxMenu.value = null;
      return true;
    }
    return false;
  }

  return { ctxMenu, openCtx, closeCtx, onWindowClick, onWindowScroll, onKeydown };
}
