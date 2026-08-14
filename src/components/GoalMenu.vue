<script setup lang="ts">
import { nextTick, onMounted, ref } from "vue";
import { setGoal, store } from "../composables/useCodex";

const emit = defineEmits<{ close: [] }>();

const text = ref(store.goalText ?? "");
const input = ref<HTMLTextAreaElement | null>(null);
/** Esc 取消后置位：阻止随后的失焦事件把输入误保存 */
const cancelled = ref(false);

async function submit() {
  const trimmed = text.value.trim();
  if (!trimmed) {
    cancelled.value = true; // 空文本视为不改动，直接关闭
    emit("close");
    return;
  }
  const ok = await setGoal(trimmed);
  if (ok) {
    cancelled.value = true; // 关闭后的 blur 不再二次提交
    emit("close");
  }
}

function onKeydown(e: KeyboardEvent) {
  // Esc 取消；Enter 提交（Shift+Enter 换行）
  if (e.key === "Escape") {
    e.preventDefault();
    cancelled.value = true;
    emit("close");
  } else if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
    e.preventDefault();
    void submit();
  }
}

/** 点击弹层外部（失焦）即确认保存；Esc 取消时不保存 */
function onBlur() {
  if (cancelled.value) return;
  void submit();
}

onMounted(() => {
  void nextTick(() => input.value?.focus());
});
</script>

<template>
  <div class="popup-menu goal-menu" @click.stop>
    <div class="menu-group-title">设置目标</div>
    <textarea
      ref="input"
      v-model="text"
      class="goal-input"
      rows="3"
      maxlength="4000"
      placeholder="输入持续追求的目标…（设置后 codex 将围绕目标自动工作）"
      @keydown="onKeydown"
      @blur="onBlur"
    ></textarea>
    <div class="goal-menu-foot">
      <span class="goal-count">{{ text.length }}/4000</span>
      <span class="goal-hint">输入后 Enter 或点击外部即生效，Esc 取消</span>
    </div>
  </div>
</template>
