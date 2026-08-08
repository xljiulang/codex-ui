<script setup lang="ts">
import { store } from "../composables/useCodex";

const emit = defineEmits<{ close: [] }>();

const TASK_MODES = [
  {
    id: "execute",
    label: "执行模式",
    desc: "直接执行任务并给出结果",
    icon: "M4 12h16M4 12l6-6M4 12l6 6",
  },
  {
    id: "plan",
    label: "计划模式",
    desc: "先制定计划再执行",
    icon: "M12 2l2.4 7.2H22l-6 4.4 2.3 7.2L12 16.5 5.7 20.8 8 13.6 2 9.2h7.6z",
  },
  {
    id: "goal",
    label: "目标模式",
    desc: "设置持续追求的目标，围绕目标工作",
    icon: "M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm0 18a8 8 0 1 1 8-8 8 8 0 0 1-8 8zm1-12.9a6 6 0 0 0-3.4 10.8l-1 1.7a8 8 0 0 1 5.8-13.2zm0 4.2a1.8 1.8 0 1 1-1.8 1.8A1.8 1.8 0 0 1 13 11.3z",
  },
] as const;

function choose(id: (typeof TASK_MODES)[number]["id"]) {
  store.taskMode = id;
  if (id === "goal") {
    store.goalOpen = true;
  }
  emit("close");
}
</script>

<template>
  <div class="popup-menu" @click.stop>
    <div class="menu-group-title" style="padding: 6px 8px">任务模式</div>
    <button
      v-for="m in TASK_MODES"
      :key="m.id"
      class="mode-menu-item"
      :class="{ selected: store.taskMode === m.id }"
      @click="choose(m.id)"
    >
      <span class="mode-icon">
        <svg viewBox="0 0 24 24">
          <path :d="m.icon" />
        </svg>
      </span>
      <span>
        <div class="mode-label">{{ m.label }}</div>
        <div class="mode-desc">{{ m.desc }}</div>
      </span>
      <span v-if="store.taskMode === m.id" class="mode-check">✓</span>
    </button>
  </div>
</template>
