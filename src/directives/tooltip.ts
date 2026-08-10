import type { Directive } from "vue";
import { hideTooltip, showTooltip } from "../composables/useTooltip";

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

type ElWithCleanup = HTMLElement & { __tooltipCleanup?: () => void };

/** 鼠标悬停显示统一的自定义 tooltip（替代原生 title） */
export const tooltipDirective: Directive<HTMLElement, unknown> = {
  mounted(el, binding) {
    const target = el as ElWithCleanup;
    target.dataset.tip = resolveText(binding.value);
    const onEnter = () =>
      showTooltip(target.dataset.tip ?? "", el.getBoundingClientRect());
    const onLeave = () => hideTooltip();
    el.addEventListener("mouseenter", onEnter);
    el.addEventListener("mouseleave", onLeave);
    target.__tooltipCleanup = () => {
      el.removeEventListener("mouseenter", onEnter);
      el.removeEventListener("mouseleave", onLeave);
    };
  },
  updated(el, binding) {
    (el as ElWithCleanup).dataset.tip = resolveText(binding.value);
  },
  unmounted(el) {
    (el as ElWithCleanup).__tooltipCleanup?.();
  },
};
