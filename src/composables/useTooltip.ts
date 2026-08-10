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
  anchor: null as TooltipAnchor | null,
});

let hideTimer: number | undefined;

export function showTooltip(text: string, rect: DOMRect) {
  const t = String(text ?? "").trim();
  if (!t) return;
  window.clearTimeout(hideTimer);
  tooltip.text = t;
  tooltip.anchor = {
    left: rect.left,
    top: rect.top,
    width: rect.width,
    height: rect.height,
  };
  tooltip.visible = true;
}

export function hideTooltip() {
  window.clearTimeout(hideTimer);
  // 轻微延迟，避免在元素间快速移动时闪烁
  hideTimer = window.setTimeout(() => {
    tooltip.visible = false;
  }, 60);
}
