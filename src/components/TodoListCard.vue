<script setup lang="ts">
import type { TodoListItem } from "../lib/types";

defineProps<{ item: TodoListItem }>();
</script>

<template>
  <div class="change-list">
    <div
      v-for="t in item.items"
      :key="t.text"
      class="todo-row"
      :class="{ done: t.completed }"
    >
      <span>{{ t.completed ? "☑" : "☐" }}</span>
      <span>{{ t.text }}</span>
    </div>
  </div>
</template>

<style scoped>
.change-list {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.todo-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 3px 0;
  font-size: var(--font-md);
}

.todo-row.done {
  color: var(--text-dim);
  text-decoration: line-through;
}
</style>
