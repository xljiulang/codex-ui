<script setup lang="ts">
import { computed, ref } from "vue";
import type { TurnPlan } from "../composables/useCodex/types";

const props = defineProps<{ plan: TurnPlan }>();

/** 默认展开：回合计划是当前进度的主视图，折叠后仅保留头部摘要 */
const open = ref(true);

/** 头部副标题：步骤进度（如 "3 步 · 1 完成"），无步骤时留空 */
const sub = computed(() => {
  const total = props.plan.steps.length;
  if (!total) return "";
  const done = props.plan.steps.filter((s) => s.status === "completed").length;
  return `${total} 步 · ${done} 完成`;
});
</script>

<template>
  <div class="assistant-card" :class="{ expanded: open }">
    <div class="assistant-card-header">
      <button
        type="button"
        class="assistant-card-toggle"
        :aria-expanded="open"
        @click="open = !open"
      >
        <span class="assistant-card-arrow">{{ open ? "▾" : "▸" }}</span>
        <span class="assistant-card-title">计划</span>
        <span v-if="sub" class="assistant-card-sub">{{ sub }}</span>
      </button>
    </div>
    <Transition name="assistant-card-body">
      <div v-if="open" class="assistant-card-body">
        <div v-if="plan.explanation" class="plan-explanation">
          {{ plan.explanation }}
        </div>
        <div
          v-for="(s, i) in plan.steps"
          :key="`${i}-${s.step}`"
          class="plan-step"
          :class="{
            pending: s.status === 'pending',
            'in-progress': s.status === 'inProgress',
            done: s.status === 'completed',
          }"
        >
          <span class="plan-step-icon">{{
            s.status === "completed" ? "☑" : s.status === "inProgress" ? "◐" : "☐"
          }}</span>
          <span class="plan-step-text">{{ s.step }}</span>
        </div>
      </div>
    </Transition>
  </div>
</template>
