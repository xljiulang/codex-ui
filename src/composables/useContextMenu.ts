import { onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { openLink } from "../lib/links";
import { clampMenuPos } from "../lib/ctxMenu";
import { copyText } from "../lib/clipboard";
import {
  ICON_CUT,
  ICON_COPY,
  ICON_PASTE,
  ICON_SELECT_ALL,
} from "../lib/icons";

export interface CtxItem {
  label: string;
  action: () => void;
  /** 菜单项图标 path（ContextMenu 组件支持渲染） */
  icon?: string;
}

export interface CtxMenuState {
  x: number;
  y: number;
  items: CtxItem[];
}

/** 会被拦截并显示自定义编辑菜单的文本输入控件 type */
const TEXT_INPUT_TYPES = new Set([
  "text",
  "search",
  "url",
  "email",
  "tel",
  "password",
]);

function isTextInput(el: HTMLElement): el is HTMLInputElement {
  return (
    el instanceof HTMLInputElement && TEXT_INPUT_TYPES.has(el.type)
  );
}

function selectionRange(el: HTMLInputElement | HTMLTextAreaElement): {
  start: number;
  end: number;
} {
  return {
    start: el.selectionStart ?? el.value.length,
    end: el.selectionEnd ?? el.value.length,
  };
}

function hasSelection(el: HTMLInputElement | HTMLTextAreaElement): boolean {
  const { start, end } = selectionRange(el);
  return start !== end;
}

function selectedText(el: HTMLInputElement | HTMLTextAreaElement): string {
  const { start, end } = selectionRange(el);
  return el.value.slice(start, end);
}

/** 在光标处插入文本并触发 input（驱动 v-model），随后把光标移到插入后位置 */
function insertAtCursor(el: HTMLInputElement | HTMLTextAreaElement, text: string) {
  const { start, end } = selectionRange(el);
  const next = el.value.slice(0, start) + text + el.value.slice(end);
  el.value = next;
  const caret = start + text.length;
  el.setSelectionRange(caret, caret);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

/** 剪切：复制选区后删除选区并触发 input（不依赖 WebView2 不支持的 execCommand("cut")） */
function cutSelection(el: HTMLInputElement | HTMLTextAreaElement) {
  const sel = selectedText(el);
  void copyText(sel);
  const { start, end } = selectionRange(el);
  el.value = el.value.slice(0, start) + el.value.slice(end);
  el.setSelectionRange(start, start);
  el.dispatchEvent(new Event("input", { bubbles: true }));
  el.focus();
}

/** 文本输入控件的编辑菜单：剪切/复制（有选区）、粘贴（走后端读剪贴板）、全选 */
function buildEditMenuItems(
  el: HTMLInputElement | HTMLTextAreaElement,
): CtxItem[] {
  const items: CtxItem[] = [];
  if (hasSelection(el)) {
    items.push({
      label: "剪切",
      icon: ICON_CUT,
      action: () => cutSelection(el),
    });
    items.push({
      label: "复制",
      icon: ICON_COPY,
      action: () => void copyText(selectedText(el)),
    });
  }
  items.push({
    label: "粘贴",
    icon: ICON_PASTE,
    action: () => {
      void invoke<string>("clipboard_read_text")
        .then((text) => {
          if (text) insertAtCursor(el, text);
        })
        .catch(() => undefined);
    },
  });
  items.push({
    label: "全选",
    icon: ICON_SELECT_ALL,
    action: () => {
      el.focus();
      el.select();
    },
  });
  return items;
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
    // 文本输入控件（input[type=text/search/...] / textarea）：统一走自定义编辑菜单，
    // 阻止 WebView2 默认菜单；粘贴经 clipboard_read_text 读系统剪贴板并插入（execCommand("paste")
    // 在 WebView2 不支持）。checkbox/radio/select/其它 contenteditable 仍交给原生。
    if (target instanceof HTMLTextAreaElement || isTextInput(target)) {
      e.preventDefault();
      const el = target as HTMLInputElement | HTMLTextAreaElement;
      const items = buildEditMenuItems(el);
      const pos = clampMenuPos(e.clientX, e.clientY, 180, items.length * 30 + 12);
      ctxMenu.value = { x: pos.x, y: pos.y, items };
      return;
    }
    const editable =
      target instanceof HTMLSelectElement ||
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement ||
      target.isContentEditable;
    // 其它可编辑元素（checkbox/radio/select/富文本编辑器）不拦截，放行给 WebView2 原生菜单。
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
