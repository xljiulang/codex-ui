<script setup lang="ts">
import { ICON_ARROW_DOWN, ICON_ARROW_RIGHT } from "../lib/icons";

defineProps<{
  label: string;
  icon: string;
  collapsed: boolean;
}>();

const emit = defineEmits<{ toggle: [] }>();
</script>

<template>
  <!-- 分区根行：与资源根节点/会话目录行同形态（箭头+图标+名称+右侧操作区），
       点击/Enter/Space 切换折叠，右键菜单由父级透传监听 -->
  <div
    class="git-section-head"
    role="button"
    tabindex="0"
    :aria-expanded="!collapsed"
    @click="emit('toggle')"
    @keydown.enter.prevent="emit('toggle')"
    @keydown.space.prevent="emit('toggle')"
  >
    <svg class="git-section-arrow" viewBox="0 0 24 24" aria-hidden="true">
      <path :d="collapsed ? ICON_ARROW_RIGHT : ICON_ARROW_DOWN" />
    </svg>
    <svg class="git-section-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path :d="icon" />
    </svg>
    <span class="git-section-main">{{ label }}</span>
    <span class="git-section-actions">
      <slot />
    </span>
  </div>
</template>
