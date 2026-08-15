<script setup lang="ts">
import type { TodoListItem } from "../lib/types";

defineProps<{ item: TodoListItem }>();
</script>

<template>
  <div class="change-list">
    <div v-for="t in item.items" :key="t.text" class="todo-row" :class="{ done: t.completed }">
      <span>{{ t.completed ? "☑" : "☐" }}</span>
      <span>{{ t.text }}</span>
    </div>
  </div>
</template>
