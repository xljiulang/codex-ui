<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { clearGoal, setGoal, store } from "../composables/useCodex";

const text = ref(store.goalText ?? "");
const textareaEl = ref<HTMLTextAreaElement | null>(null);
let lastFocus: HTMLElement | null = null;

function save() {
  if (store.turnActive) return;
  void setGoal(text.value.trim());
}

function clear() {
  if (store.turnActive) return;
  void clearGoal();
  store.goalOpen = false;
  text.value = "";
}

function cancel() {
  // 未设置目标时取消，则回到执行模式
  if (store.taskMode === "goal" && !store.goalText) {
    store.taskMode = "execute";
  }
  store.goalOpen = false;
  text.value = store.goalText ?? "";
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") cancel();
}

onMounted(() => {
  lastFocus = document.activeElement as HTMLElement | null;
  void nextTick(() => textareaEl.value?.focus());
  window.addEventListener("keydown", onKeydown);
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  lastFocus?.focus?.();
});
</script>

<template>
  <div class="modal-mask">
    <div class="modal">
      <div class="modal-head">
        <span class="modal-title">设置目标</span>
        <button class="modal-close" @click="cancel()">×</button>
      </div>
      <div class="modal-body">
        <textarea
          ref="textareaEl"
          v-model="text"
          rows="4"
          style="width: 100%"
          placeholder="描述要持续追求的目标…"
        ></textarea>
      </div>
      <div class="modal-foot">
        <button class="btn" @click="clear()">清除目标</button>
        <button class="btn" @click="cancel()">取消</button>
        <button class="btn primary" :disabled="!text.trim()" @click="save()">保存</button>
      </div>
    </div>
  </div>
</template>
