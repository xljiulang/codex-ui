<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, useAttrs } from "vue";

const props = defineProps<{
  modelValue: string;
  options: string[];
  disabled?: boolean;
  error?: boolean;
}>();

// 非 prop 透传（如 id）落到内部 input，便于 label[for] 关联聚焦
defineOptions({ inheritAttrs: false });

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
}>();

const attrs = useAttrs();
const open = ref(false);
const anchorEl = ref<HTMLElement | null>(null);

/** 点击/聚焦输入框即展开全量候选（不做按输入值过滤） */
function openMenu() {
  if (props.disabled) return;
  open.value = true;
}

function onInput(e: Event) {
  emit("update:modelValue", (e.target as HTMLInputElement).value);
}

/** 选中某候选：回填其 id/slug 并收起 */
function selectOption(opt: string) {
  emit("update:modelValue", opt);
  open.value = false;
}

/** 点击锚点（输入框/弹层）内部不关闭，点击外部才收起 */
function onWindowMousedown(e: MouseEvent) {
  if (!anchorEl.value) return;
  if (anchorEl.value.contains(e.target as Node)) return;
  open.value = false;
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") open.value = false;
}

onMounted(() => {
  window.addEventListener("mousedown", onWindowMousedown);
  window.addEventListener("keydown", onKeydown);
});

onBeforeUnmount(() => {
  window.removeEventListener("mousedown", onWindowMousedown);
  window.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <div ref="anchorEl" class="menu-anchor model-config-model-picker">
    <input
      v-bind="attrs"
      :value="modelValue"
      type="text"
      autocomplete="off"
      :disabled="disabled"
      placeholder="如 deepseek-v4-flash"
      :class="{ 'model-config-input-error': error }"
      @focus="openMenu"
      @click="openMenu"
      @input="onInput"
    />
    <div
      v-if="open"
      class="popup-menu model-config-model-picker-menu"
      @click.stop
    >
      <div
        v-if="options.length"
        class="question-options"
        style="padding: 0 8px 4px"
      >
        <button
          v-for="opt in options"
          :key="opt"
          type="button"
          class="option-btn"
          :class="{ selected: opt === modelValue }"
          @click="selectOption(opt)"
        >
          <span>{{ opt }}</span>
        </button>
      </div>
      <div v-else class="menu-note">无可选模型</div>
    </div>
  </div>
</template>

<style scoped>
/* 候选列表向下弹出（覆盖 .popup-menu 默认 bottom 向上定位） */
.model-config-model-picker-menu {
  top: calc(100% + 8px);
  bottom: auto;
}
</style>
