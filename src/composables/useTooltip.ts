import { reactive } from "vue";

export interface TooltipAnchor {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** 全局 tooltip 单例状态：由 v-tooltip 指令驱动，TooltipLayer 负责渲染 */
export const tooltip = reactive({
  visible: false,
  text: "",
  x: 0,
  y: 0,
  placement: "top" as "top" | "left",
  anchor: null as TooltipAnchor | null,
  /** 当前 tooltip 的归属元素（指令悬停目标），用于隐藏/移除时兜底清理 */
  anchorEl: null as HTMLElement | null,
});

let hideTimer: number | undefined;
let pointerGuard: ((e: MouseEvent) => void) | null = null;

/** 元素是否可见：已断开、或计算样式 display/visibility 隐藏视为不可见 */
function isElementVisible(el: HTMLElement): boolean {
  if (!el.isConnected) return false;
  const cs = getComputedStyle(el);
  return cs.display !== "none" && cs.visibility !== "hidden";
}

/** 全局指针守卫：锚点元素被隐藏/移除，或指针移出其实时包围盒时兜底隐藏 */
function ensurePointerGuard() {
  if (pointerGuard) return;
  pointerGuard = (e: MouseEvent) => {
    const el = tooltip.anchorEl;
    if (!el || !tooltip.visible) return;
    if (!isElementVisible(el)) {
      hideTooltip();
      return;
    }
    const rect = el.getBoundingClientRect();
    const pad = 4;
    if (
      e.clientX < rect.left - pad ||
      e.clientX > rect.right + pad ||
      e.clientY < rect.top - pad ||
      e.clientY > rect.bottom + pad
    ) {
      hideTooltip();
    }
  };
  window.addEventListener("mousemove", pointerGuard);
}

function removePointerGuard() {
  if (!pointerGuard) return;
  window.removeEventListener("mousemove", pointerGuard);
  pointerGuard = null;
}

export function showTooltip(
  text: string,
  anchor: TooltipAnchor,
  el?: HTMLElement | null,
  placement: "top" | "left" = "top",
) {
  const t = String(text ?? "").trim();
  if (!t) return;
  window.clearTimeout(hideTimer);
  tooltip.text = t;
  tooltip.placement = placement;
  tooltip.anchor = {
    left: anchor.left,
    top: anchor.top,
    width: anchor.width,
    height: anchor.height,
  };
  tooltip.anchorEl = el ?? null;
  tooltip.visible = true;
  if (el) ensurePointerGuard();
}

export function hideTooltip() {
  window.clearTimeout(hideTimer);
  // 轻微延迟，避免在元素间快速移动时闪烁
  hideTimer = window.setTimeout(() => {
    tooltip.visible = false;
    tooltip.anchor = null;
    tooltip.anchorEl = null;
    tooltip.placement = "top";
    removePointerGuard();
  }, 60);
}
