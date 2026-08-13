<script setup lang="ts">
import { defineAsyncComponent, onBeforeUnmount, onMounted } from "vue";
import AppHeader from "./components/AppHeader.vue";
import RightPanel from "./components/RightPanel.vue";
import ChatView from "./components/ChatView.vue";
import SettingsView from "./components/SettingsView.vue";
import InteractionDialog from "./components/InteractionDialog.vue";
import PlanDialog from "./components/PlanDialog.vue";
import ConfirmDialog from "./components/ConfirmDialog.vue";
import DiffWindowView from "./components/DiffWindowView.vue";
import TooltipLayer from "./components/TooltipLayer.vue";
import { disposeEvents, init, store } from "./composables/useCodex";
import { registerCloseGuard } from "./composables/useCloseGuard";
import { useContextMenu } from "./composables/useContextMenu";
import { getCurrentWindow } from "@tauri-apps/api/window";

// 文本编辑器（CodeMirror）按需加载：仅打开编辑器窗口时才下载对应 chunk
const TextEditorWindow = defineAsyncComponent(
  () => import("./components/TextEditorWindow.vue"),
);

// 独立窗口（diff / 文本预览）：按窗口 label 识别（不依赖 localStorage，避免标志残留污染下次启动）
let isDiffWindow = false;
let isTextWindow = false;
try {
  const label = getCurrentWindow().label;
  isDiffWindow = label === "diff-preview";
  isTextWindow = label === "text-editor";
} catch {
  // 非 Tauri 环境（如单测）按普通窗口处理
}
const isStandaloneWindow = isDiffWindow || isTextWindow;

// 主窗口注册自定义右键菜单（独立窗口由各自组件自行注册）
const { ctxMenu } = useContextMenu(!isStandaloneWindow);

let unlistenClose: (() => void) | undefined;

onMounted(async () => {
  if (isStandaloneWindow) return;
  unlistenClose = await registerCloseGuard();
  void init();
});

onBeforeUnmount(() => {
  unlistenClose?.();
  if (isStandaloneWindow) return;
  disposeEvents();
});
</script>

<template>
  <TextEditorWindow v-if="isTextWindow" />
  <DiffWindowView v-else-if="isDiffWindow" />
  <div v-else class="app">
    <AppHeader />
    <div class="app-body">
      <ChatView />
      <RightPanel />
    </div>
    <SettingsView v-if="store.showSettings" />
    <InteractionDialog />
    <PlanDialog />
    <ConfirmDialog />
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
  <TooltipLayer />
</template>
