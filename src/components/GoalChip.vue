<script setup lang="ts">
import { clearGoal, store } from "../composables/useCodex";
import { ICON_GOAL } from "../lib/icons";

/** 目标预览：压缩连续空白并截断到 120 字符，超长追加省略号 */
function goalPreview(text: string): string {
  const t = text.replace(/\s+/g, " ").trim();
  return t.length > 120 ? `${t.slice(0, 120)}…` : t;
}

/** 目标旗子提示：未勾选/已勾选待首条消息/已挂载（进行中） */
function goalTooltip(): string {
  if (!store.goalText && !store.goalArmed) {
    return "目标：勾选后，首条消息将作为目标自动执行";
  }
  if (store.goalArmed && !store.goalText) {
    return "目标已勾选：发送首条消息后自动以其为目标，点击取消勾选";
  }
  const text = store.goalText ?? "";
  if (!store.currentThreadId) return `目标：${goalPreview(text)}（待挂载）`;
  return `目标：${goalPreview(text)}（进行中），点击取消目标`;
}

/** 目标旗子：已挂载/回填目标时点击直接取消（clearGoal）；否则切换勾选态 */
function onGoalIconClick() {
  if (store.goalText) {
    void clearGoal();
    return;
  }
  store.goalArmed = !store.goalArmed;
}
</script>

<template>
  <div
    v-if="!store.turnActive || !!store.goalText"
    class="goal-chip"
    :class="{ 'has-goal': !!store.goalText || store.goalArmed }"
  >
    <button
      class="goal-icon-btn"
      :class="{
        'has-goal': !!store.goalText || store.goalArmed,
        'status-active':
          !!store.goalText &&
          (store.goalStatus === 'active' || store.goalStatus === null),
      }"
      :aria-pressed="!!store.goalText || store.goalArmed"
      :aria-label="goalTooltip()"
      v-tooltip="goalTooltip()"
      @click="onGoalIconClick()"
    >
      <svg viewBox="0 0 24 24">
        <path :d="ICON_GOAL" />
      </svg>
      <span
        v-if="store.goalArmed || store.goalText"
        class="goal-check"
        aria-hidden="true"
      ></span>
    </button>
  </div>
</template>
