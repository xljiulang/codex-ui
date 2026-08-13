import { onBeforeUnmount, onMounted, ref } from "vue";
import { openLink } from "../lib/links";
import { clampMenuPos } from "../lib/ctxMenu";
import { copyText } from "../lib/clipboard";

export interface CtxItem {
  label: string;
  action: () => void;
}

export interface CtxMenuState {
  x: number;
  y: number;
  items: CtxItem[];
}

/**
 * 自定义右键菜单（阻止默认菜单，避免页面刷新）：
 * 可编辑元素（输入框/文本域/富文本编辑器）不拦截，交给 WebView2 原生菜单（原生粘贴可用）；
 * 非编辑区 → 选中文本复制、链接 → 打开链接/复制链接地址。
 */
export function useContextMenu(enabled = true, alwaysCopy = false) {
  const ctxMenu = ref<CtxMenuState | null>(null);

  function onContextMenu(e: MouseEvent) {
    const target = e.target as HTMLElement;
    const editable =
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement ||
      target.isContentEditable;
    // 可编辑元素（输入框/文本域/富文本编辑器）不拦截右键，让 WebView2 原生菜单处理：
    // document.execCommand("paste") 在 Chromium/WebView2 中不支持，自定义菜单的“粘贴”是
    // 无效项；原生菜单的粘贴与 Ctrl+V 走同一路径。非编辑区继续使用自定义菜单。
    if (editable) {
      ctxMenu.value = null;
      return;
    }
    e.preventDefault();
    const selection = window.getSelection()?.toString() ?? "";
    const link = target.closest("a") as HTMLAnchorElement | null;
    const items: CtxItem[] = [];
    if (selection || alwaysCopy) {
      items.push({
        label: "复制",
        action: () => {
          if (selection) {
            void copyText(selection);
          } else {
            // 无选区时复制整行内容（diff 行为 .diff-text）
            const line = (target.closest?.(".diff-text") ?? target) as
              | HTMLElement
              | null;
            void copyText(line?.innerText ?? target.textContent ?? "");
          }
        },
      });
    }
    if (link) {
      items.push({
        label: "打开链接",
        action: () => openLink(link.href),
      });
      items.push({
        label: "复制链接地址",
        action: () => void copyText(link.href),
      });
    }
    if (!items.length) {
      ctxMenu.value = null;
      return;
    }
    const pos = clampMenuPos(
      e.clientX,
      e.clientY,
      160,
      items.length * 30 + 12,
    );
    ctxMenu.value = { x: pos.x, y: pos.y, items };
  }

  function onGlobalClick() {
    ctxMenu.value = null;
  }

  function onGlobalKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") ctxMenu.value = null;
  }

  function onGlobalScroll() {
    ctxMenu.value = null;
  }

  onMounted(() => {
    if (!enabled) return;
    window.addEventListener("contextmenu", onContextMenu);
    window.addEventListener("click", onGlobalClick);
    window.addEventListener("keydown", onGlobalKeydown);
    window.addEventListener("scroll", onGlobalScroll, true);
  });

  onBeforeUnmount(() => {
    window.removeEventListener("contextmenu", onContextMenu);
    window.removeEventListener("click", onGlobalClick);
    window.removeEventListener("keydown", onGlobalKeydown);
    window.removeEventListener("scroll", onGlobalScroll, true);
  });

  return { ctxMenu };
}
