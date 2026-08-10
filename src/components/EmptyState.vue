<script setup lang="ts">
defineProps<{ busy: boolean }>();
</script>

<template>
  <div class="empty-state">
    <div class="empty-logo" :class="{ busy }">
      <svg viewBox="0 0 24 24">
        <path d="M12 2l8.66 5v10L12 22l-8.66-5V7z" />
        <path class="logo-c" d="M14.9 9.1a4.5 4.5 0 1 0 0 5.8" />
      </svg>
    </div>
    <span v-if="busy">正在处理请求…</span>
    <span v-else>输入消息开始新的对话</span>
  </div>
</template>
