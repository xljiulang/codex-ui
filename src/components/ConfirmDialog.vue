<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { settleConfirm, store } from "../composables/useCodex";

const confirmBtn = ref<HTMLButtonElement | null>(null);

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") settleConfirm(false);
}

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
  void nextTick(() => confirmBtn.value?.focus());
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <div v-if="store.confirm" class="modal-mask">
    <div
      class="modal"
      role="alertdialog"
      aria-modal="true"
      :aria-label="store.confirm.title"
    >
      <div class="modal-head">
        <span class="modal-title">{{ store.confirm.title }}</span>
      </div>
      <div class="modal-body">{{ store.confirm.message }}</div>
      <div class="modal-foot">
        <button class="btn" @click="settleConfirm(false)">
          {{ store.confirm.cancelLabel ?? "取消" }}
        </button>
        <button
          ref="confirmBtn"
          class="btn danger"
          @click="settleConfirm(true)"
        >
          {{ store.confirm.confirmLabel ?? "确认" }}
        </button>
      </div>
    </div>
  </div>
</template>
