<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import MarkdownText from "./MarkdownText.vue";
import {
  dismissPlanPrompt,
  executePlan,
  exitPlanMode,
  store,
} from "../composables/useCodex";

const modalEl = ref<HTMLElement | null>(null);
const primaryBtn = ref<HTMLButtonElement | null>(null);
let lastFocus: HTMLElement | null = null;

// 弹窗打开时把焦点移入（首个主按钮），关闭时还原到之前的焦点
watch(
  () => store.planPrompt,
  (v) => {
    if (v) {
      lastFocus = document.activeElement as HTMLElement | null;
      void nextTick(() => {
        (primaryBtn.value ?? modalEl.value)?.focus();
      });
    } else if (lastFocus) {
      lastFocus.focus?.();
      lastFocus = null;
    }
  },
);

function onKeydown(e: KeyboardEvent) {
  // Esc 等同“待在计划”：关闭弹窗、保持计划模式、不发消息
  if (e.key === "Escape" && store.planPrompt) dismissPlanPrompt();
}

onMounted(() => window.addEventListener("keydown", onKeydown));
onBeforeUnmount(() => window.removeEventListener("keydown", onKeydown));
</script>

<template>
  <div v-if="store.planPrompt" class="modal-mask">
    <div
      ref="modalEl"
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-label="计划已就绪"
      tabindex="-1"
    >
      <div class="modal-head">
        <span class="modal-title">计划已就绪</span>
      </div>
      <div class="modal-body plan-prompt-body">
        <div class="plan-prompt-hint">计划已生成，确认后即可开始执行；聊天流中保留完整计划。</div>
        <div class="plan-prompt-content">
          <MarkdownText :text="store.planPrompt.planText" />
        </div>
      </div>
      <div class="modal-foot">
        <button class="btn" @click="dismissPlanPrompt()">待在计划</button>
        <button class="btn" @click="exitPlanMode()">退出计划模式</button>
        <button ref="primaryBtn" class="btn primary" @click="executePlan()">执行计划</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.plan-prompt-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.plan-prompt-hint {
  font-size: 12px;
  color: var(--text-dim);
}

.plan-prompt-content {
  max-height: 42vh;
  overflow: auto;
  border: 1px solid var(--border-light);
  border-radius: 8px;
  padding: 10px 12px;
  font-size: 13px;
  line-height: 1.6;
}
</style>
