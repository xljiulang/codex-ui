<script setup lang="ts">
defineProps<{ busy: boolean }>();
</script>

<template>
  <div class="empty-state">
    <div v-if="busy" class="hexagon"></div>
    <div v-else class="hexagon" style="animation: none; opacity: 0.4"></div>
    <span v-if="busy">正在处理请求…</span>
    <span v-else>输入消息开始新的对话</span>
  </div>
</template>
