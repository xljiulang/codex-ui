import type { Directive } from "vue";
import { hideTooltip, showTooltip, tooltip } from "../composables/useTooltip";

function resolveText(value: unknown): string {
  if (typeof value === "string") return value;
  if (
    value &&
    typeof value === "object" &&
    "text" in (value as Record<string, unknown>)
  ) {
    return String((value as { text: unknown }).text ?? "");
  }
  return "";
}

type ElWithCleanup = HTMLElement & {
  __tooltipCleanup?: () => void;
  __tooltipObserver?: MutationObserver | null;
};

/** 元素是否可见（与 useTooltip 内判断一致）：display/visibility 隐藏视为不可见 */
function isElementVisible(el: HTMLElement): boolean {
  if (!el.isConnected) return false;
  const cs = getComputedStyle(el);
  return cs.display !== "none" && cs.visibility !== "hidden";
}

/** 鼠标悬停显示统一的自定义 tooltip（替代原生 title） */
export const tooltipDirective: Directive<HTMLElement, unknown> = {
  mounted(el, binding) {
    const target = el as ElWithCleanup;
    target.dataset.tip = resolveText(binding.value);
    const onEnter = () =>
      showTooltip(target.dataset.tip ?? "", el.getBoundingClientRect(), el);
    const onLeave = () => hideTooltip();
    el.addEventListener("mouseenter", onEnter);
    el.addEventListener("mouseleave", onLeave);
    // 元素自身被隐藏（v-show 等改 style/class）时立即收起 tooltip，
    // 避免 mouseleave 不触发导致的残留
    if (typeof MutationObserver !== "undefined") {
      target.__tooltipObserver = new MutationObserver(() => {
        if (tooltip.anchorEl === el && !isElementVisible(el)) hideTooltip();
      });
      target.__tooltipObserver.observe(el, {
        attributes: true,
        attributeFilter: ["style", "class"],
      });
    }
    target.__tooltipCleanup = () => {
      el.removeEventListener("mouseenter", onEnter);
      el.removeEventListener("mouseleave", onLeave);
      target.__tooltipObserver?.disconnect();
      target.__tooltipObserver = null;
    };
  },
  updated(el, binding) {
    (el as ElWithCleanup).dataset.tip = resolveText(binding.value);
  },
  unmounted(el) {
    const target = el as ElWithCleanup;
    target.__tooltipCleanup?.();
    // v-if 移除时 mouseleave 不触发：若 tooltip 正归属该元素则收起
    if (tooltip.anchorEl === el) hideTooltip();
  },
};
