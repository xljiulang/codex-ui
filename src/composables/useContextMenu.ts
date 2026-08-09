import { onBeforeUnmount, onMounted, ref } from "vue";
import { openLink } from "../lib/links";

export interface CtxItem {
  label: string;
  action: () => void;
}

export interface CtxMenuState {
  x: number;
  y: number;
  items: CtxItem[];
}

async function copyText(t: string) {
  try {
    await navigator.clipboard.writeText(t);
  } catch {
    try {
      const ta = document.createElement("textarea");
      ta.value = t;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      document.body.removeChild(ta);
    } catch {
      // ignore
    }
  }
}

/**
 * 自定义右键菜单（阻止默认菜单，避免页面刷新）：
 * 可编辑元素 → 剪切/复制/粘贴/全选；选中文本 → 复制；链接 → 打开链接/复制链接地址。
 */
export function useContextMenu(enabled = true, alwaysCopy = false) {
  const ctxMenu = ref<CtxMenuState | null>(null);

  function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    const target = e.target as HTMLElement;
    const editable =
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement ||
      target.isContentEditable;
    const selection = window.getSelection()?.toString() ?? "";
    const link = target.closest("a") as HTMLAnchorElement | null;
    const items: CtxItem[] = [];
    if (editable) {
      items.push(
        { label: "剪切", action: () => document.execCommand("cut") },
        { label: "复制", action: () => document.execCommand("copy") },
        { label: "粘贴", action: () => document.execCommand("paste") },
        { label: "全选", action: () => document.execCommand("selectAll") },
      );
    } else if (selection || alwaysCopy) {
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
    const x = Math.min(e.clientX, window.innerWidth - 160);
    const y = Math.min(e.clientY, window.innerHeight - items.length * 30 - 12);
    ctxMenu.value = { x, y, items };
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
