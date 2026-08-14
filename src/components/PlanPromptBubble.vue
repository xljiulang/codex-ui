<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  dismissPlanPrompt,
  executePlan,
  exitPlanMode,
  store,
} from "../composables/useCodex";
import { focusComposer } from "../lib/composerFocus";

const bubbleEl = ref<HTMLElement | null>(null);

// 出现时聚焦“执行计划”主按钮，解决后把焦点还给输入框
watch(
  () => store.planPrompt,
  (v, prev) => {
    if (v) {
      void nextTick(() => {
        const primary = bubbleEl.value?.querySelector<HTMLElement>(
          ".interaction-foot .btn.primary",
        );
        (primary ?? bubbleEl.value)?.focus();
      });
    } else if (prev) {
      focusComposer();
    }
  },
);

function onKeydown(e: KeyboardEvent) {
  // Esc 等同“待在计划”：关闭气泡、保持计划模式、不发消息
  if (e.key === "Escape" && store.planPrompt) dismissPlanPrompt();
}

onMounted(() => window.addEventListener("keydown", onKeydown));
onBeforeUnmount(() => window.removeEventListener("keydown", onKeydown));
</script>

<template>
  <div v-if="store.planPrompt" class="msg msg-agent">
    <div ref="bubbleEl" class="interaction-bubble" tabindex="-1">
      <div class="interaction-head">
        <span class="interaction-title">计划已就绪</span>
      </div>
      <div class="interaction-body">
        <div class="plan-prompt-hint">
          计划已生成，确认后即可开始执行；完整计划见上方消息。
        </div>
      </div>
      <div class="interaction-foot">
        <button class="btn" @click="dismissPlanPrompt()">待在计划</button>
        <button class="btn" @click="exitPlanMode()">退出计划模式</button>
        <button class="btn primary" @click="executePlan()">执行计划</button>
      </div>
    </div>
  </div>
</template>
