<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import AppHeader from "./components/AppHeader.vue";
import HistoryPanel from "./components/HistoryPanel.vue";
import ChatView from "./components/ChatView.vue";
import SettingsView from "./components/SettingsView.vue";
import InteractionDialog from "./components/InteractionDialog.vue";
import GoalDialog from "./components/GoalDialog.vue";
import DiffWindowView from "./components/DiffWindowView.vue";
import { disposeEvents, init, store } from "./composables/useCodex";
import { useContextMenu } from "./composables/useContextMenu";
import { getCurrentWindow } from "@tauri-apps/api/window";

// 独立 diff 窗口：按窗口 label 识别（不依赖 localStorage，避免标志残留污染下次启动）
let isDiffWindow = false;
try {
  isDiffWindow = getCurrentWindow().label === "diff-preview";
} catch {
  // 非 Tauri 环境（如单测）按普通窗口处理
}

// 主窗口注册自定义右键菜单（diff 窗口由 DiffWindowView 自行注册）
const { ctxMenu } = useContextMenu(!isDiffWindow);

onMounted(() => {
  if (isDiffWindow) return;
  void init();
});

onBeforeUnmount(() => {
  if (isDiffWindow) return;
  disposeEvents();
});
</script>

<template>
  <DiffWindowView v-if="isDiffWindow" />
  <div v-else class="app">
    <AppHeader />
    <div class="app-body">
      <HistoryPanel v-if="store.showHistory" />
      <SettingsView v-if="store.showSettings" />
      <ChatView v-else />
    </div>
    <InteractionDialog />
    <GoalDialog v-if="store.goalOpen" />
    <div v-if="store.toast" class="toast">{{ store.toast }}</div>
    <div
      v-if="ctxMenu"
      class="ctx-menu"
      :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }"
      @click.stop
    >
      <button
        v-for="it in ctxMenu.items"
        :key="it.label"
        class="ctx-menu-item"
        @click="it.action(); ctxMenu = null"
      >
        {{ it.label }}
      </button>
    </div>
  </div>
</template>
