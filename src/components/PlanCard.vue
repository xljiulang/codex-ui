<script setup lang="ts">
import type { TurnPlan } from "../composables/useCodex/types";

defineProps<{ plan: TurnPlan }>();
</script>

<template>
  <div class="plan-card">
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
</template>
