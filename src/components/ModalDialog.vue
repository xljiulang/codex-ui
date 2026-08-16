<script setup lang="ts">
import { ref, type Ref } from "vue";

const props = withDefaults(
  defineProps<{
    title: string;
    /** 头部显示关闭按钮（×） */
    closable?: boolean;
    /** 点击遮罩关闭 */
    maskClose?: boolean;
    /** 附加到 body 容器上的类（如 resource-props） */
    bodyClass?: string;
    /** 可选：把内部 modal 元素暴露给父级（聚焦等 DOM 操作） */
    rootRef?: Ref<HTMLElement | null>;
  }>(),
  { closable: false, maskClose: false, bodyClass: "" },
);

const emit = defineEmits<{ close: [] }>();
const modalEl = ref<HTMLElement | null>(null);

function bindModalEl(el: unknown) {
  modalEl.value = (el as HTMLElement | null) ?? null;
  if (props.rootRef) props.rootRef.value = modalEl.value;
}
</script>

<template>
  <div class="modal-mask" @click.self="maskClose ? emit('close') : undefined">
    <div
      :ref="bindModalEl"
      class="modal"
      tabindex="-1"
      role="dialog"
      aria-modal="true"
    >
      <div class="modal-head">
        <span class="modal-title">{{ title }}</span>
        <button
          v-if="closable"
          class="modal-close"
          aria-label="关闭"
          @click="emit('close')"
        >
          ×
        </button>
      </div>
      <div class="modal-body" :class="bodyClass">
        <slot />
      </div>
      <div v-if="$slots.foot" class="modal-foot">
        <slot name="foot" />
      </div>
    </div>
  </div>
</template>
