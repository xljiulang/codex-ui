<script lang="ts">
/** AppSelect 选项：value 为写回 modelValue 的值，label 为展示文案 */
export interface AppSelectOption {
  value: string;
  label: string;
}
</script>

<script setup lang="ts">
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  ref,
  useAttrs,
} from "vue";
import { clampMenuPos } from "../lib/ctxMenu";
import { ICON_SELECT_CHEVRON } from "../lib/icons";

const props = defineProps<{
  modelValue: string;
  options: AppSelectOption[];
  disabled?: boolean;
  /** 无匹配值时按钮上显示的占位文案（如「请选择」） */
  placeholder?: string;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
}>();

// 非 prop 透传（如 id、class）落到触发按钮，保持 label[for] 关联聚焦
defineOptions({ inheritAttrs: false });

const attrs = useAttrs();
const triggerEl = ref<HTMLElement | null>(null);
const menuEl = ref<HTMLElement | null>(null);
const open = ref(false);
const highlighted = ref(-1);
// 弹层 Teleport 到 body + fixed 定位：不受 .modal（overflow:hidden）裁剪
const menuLeft = ref(0);
const menuTop = ref(0);
const menuMinWidth = ref(0);
const menuMaxHeight = ref(240);

const matched = computed(() =>
  props.options.some((o) => o.value === props.modelValue),
);
/** 按钮文案：选中项 label；无匹配时退回 placeholder / 原始值 */
const currentLabel = computed(
  () =>
    props.options.find((o) => o.value === props.modelValue)?.label ??
    props.placeholder ??
    props.modelValue,
);

/** 弹层期望高度估算（选项行高约 30px），用于翻转判断与限高 */
function estimateMenuHeight() {
  return props.options.length * 30 + 8;
}

/** 依触发按钮计算 fixed 坐标：优先向下弹出，下方放不下且上方更宽裕时向上翻 */
function syncMenuPos() {
  const rect = triggerEl.value?.getBoundingClientRect();
  if (!rect) return;
  menuMinWidth.value = Math.round(rect.width);
  const estH = estimateMenuHeight();
  const spaceBelow = window.innerHeight - rect.bottom - 8;
  const spaceAbove = rect.top - 8;
  const below = spaceBelow >= Math.min(estH, 200) || spaceBelow >= spaceAbove;
  const menuH = Math.max(
    Math.min(estH, Math.max(below ? spaceBelow : spaceAbove, 120)),
    120,
  );
  menuMaxHeight.value = menuH;
  const top = below ? rect.bottom + 4 : rect.top - 4 - menuH;
  const pos = clampMenuPos(rect.left, top, Math.max(rect.width, 180), menuH);
  menuLeft.value = pos.x;
  menuTop.value = pos.y;
}

function openMenu() {
  if (props.disabled || open.value) return;
  highlighted.value = props.options.findIndex(
    (o) => o.value === props.modelValue,
  );
  syncMenuPos();
  open.value = true;
  // 展开后把选中项滚进可视区
  void nextTick(() => {
    menuEl.value
      ?.querySelector<HTMLElement>(".app-select-option.selected")
      ?.scrollIntoView?.({ block: "nearest" });
  });
}

function toggleMenu() {
  if (open.value) closeMenu();
  else openMenu();
}

function closeMenu() {
  open.value = false;
}

function choose(opt: AppSelectOption) {
  emit("update:modelValue", opt.value);
  closeMenu();
  triggerEl.value?.focus();
}

function onTriggerKeydown(e: KeyboardEvent) {
  if (props.disabled) return;
  if (!open.value) {
    if (["Enter", " ", "ArrowDown", "ArrowUp"].includes(e.key)) {
      e.preventDefault();
      openMenu();
    }
    return;
  }
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    const n = props.options.length;
    if (!n) return;
    const from =
      highlighted.value < 0
        ? e.key === "ArrowDown"
          ? -1
          : 0
        : highlighted.value;
    highlighted.value = (from + (e.key === "ArrowDown" ? 1 : -1) + n) % n;
  } else if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    const opt = props.options[highlighted.value];
    if (opt) choose(opt);
  }
}

function onWindowMousedown(e: MouseEvent) {
  if (!open.value) return;
  const target = e.target as HTMLElement | null;
  if (triggerEl.value?.contains(target as Node)) return;
  if (target?.closest?.(".app-select-menu")) return;
  closeMenu();
}

function onWindowKeydown(e: KeyboardEvent) {
  if (open.value && e.key === "Escape") {
    closeMenu();
    triggerEl.value?.focus();
  }
}

/** 视口滚动/缩放后锚点位置失效：关闭弹层（弹层自身滚动除外） */
function onWindowScrollOrResize(e: Event) {
  if (!open.value) return;
  const target = e.target as HTMLElement | null;
  if (e.type === "scroll" && target?.closest?.(".app-select-menu")) return;
  closeMenu();
}

onMounted(() => {
  window.addEventListener("mousedown", onWindowMousedown);
  window.addEventListener("keydown", onWindowKeydown);
  window.addEventListener("scroll", onWindowScrollOrResize, true);
  window.addEventListener("resize", onWindowScrollOrResize);
});

onBeforeUnmount(() => {
  window.removeEventListener("mousedown", onWindowMousedown);
  window.removeEventListener("keydown", onWindowKeydown);
  window.removeEventListener("scroll", onWindowScrollOrResize, true);
  window.removeEventListener("resize", onWindowScrollOrResize);
});
</script>

<template>
  <button
    ref="triggerEl"
    type="button"
    class="app-select"
    role="combobox"
    aria-haspopup="listbox"
    :aria-expanded="open ? 'true' : 'false'"
    :disabled="disabled"
    v-bind="attrs"
    @click="toggleMenu"
    @keydown="onTriggerKeydown"
  >
    <span
      class="app-select-label"
      :class="{ 'is-placeholder': !matched && !!placeholder }"
    >
      {{ currentLabel }}
    </span>
    <svg class="app-select-chevron" viewBox="0 0 24 24" aria-hidden="true">
      <path :d="ICON_SELECT_CHEVRON" />
    </svg>
  </button>
  <Teleport to="body">
    <div
      v-if="open"
      ref="menuEl"
      class="app-select-menu"
      role="listbox"
      :style="{
        left: menuLeft + 'px',
        top: menuTop + 'px',
        minWidth: menuMinWidth + 'px',
        maxHeight: menuMaxHeight + 'px',
      }"
    >
      <button
        v-for="(opt, i) in options"
        :key="opt.value"
        type="button"
        role="option"
        class="app-select-option"
        :class="{
          selected: opt.value === modelValue,
          highlighted: i === highlighted,
        }"
        :aria-selected="opt.value === modelValue ? 'true' : 'false'"
        @click="choose(opt)"
        @mousemove="highlighted = i"
      >
        {{ opt.label }}
      </button>
    </div>
  </Teleport>
</template>

<style scoped>
.app-select {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  width: 100%;
  height: var(--ctrl-h-md);
  padding: var(--space-2) var(--space-4);
  font: inherit;
  font-size: var(--font-md);
  color: var(--text-bright);
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  text-align: left;
  cursor: pointer;
}

.app-select:hover:not(:disabled),
.app-select:focus-visible {
  border-color: var(--accent-dim);
}

.app-select:disabled {
  opacity: 0.5;
  cursor: default;
}

.app-select-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.app-select-label.is-placeholder {
  color: var(--text-faint);
}

.app-select-chevron {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  fill: currentColor;
  color: var(--text-faint);
  transition: transform var(--ease);
}

.app-select[aria-expanded="true"] .app-select-chevron {
  transform: rotate(180deg);
}

.app-select-menu {
  position: fixed;
  z-index: 140;
  min-width: 160px;
  padding: var(--space-1);
  background: var(--float-bg);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-lg);
  overflow-y: auto;
  animation: menu-in var(--ease);
}

.app-select-option {
  display: block;
  width: 100%;
  padding: var(--space-1) var(--space-2);
  border: 1px solid transparent;
  border-radius: var(--radius);
  background: transparent;
  color: var(--text-dim);
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.app-select-option.highlighted {
  background: rgba(var(--accent-rgb), 0.06);
  border-color: rgba(var(--accent-rgb), 0.14);
}

.app-select-option.selected {
  color: var(--text-bright);
  background: rgba(var(--accent-rgb), 0.12);
  border-color: rgba(var(--accent-rgb), 0.28);
}
</style>
