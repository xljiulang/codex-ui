<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import AppHeader from "./components/AppHeader.vue";
import ContextMenu from "./components/ContextMenu.vue";
import EditorPane from "./components/EditorPane.vue";
import RightPanel from "./components/RightPanel.vue";
import LoadingScreen from "./components/LoadingScreen.vue";
import ConfirmDialog from "./components/ConfirmDialog.vue";
import TooltipLayer from "./components/TooltipLayer.vue";
import { disposeEvents, init, store } from "./composables/useCodex";
import { registerCloseGuard } from "./composables/useCloseGuard";
import { useContextMenu } from "./composables/useContextMenu";
import { openSettingsTab } from "./composables/useEditorTabs";

const { ctxMenu } = useContextMenu();

let unlistenClose: (() => void) | undefined;

onMounted(async () => {
  // 关闭守卫仅阻止 Tauri 默认销毁窗口；关闭即隐藏到系统托盘由 Rust 端处理
  unlistenClose = await registerCloseGuard();
  try {
  await init();
  } finally {
    // 启动无条件打开设置页：每次启动创建并激活（设置 tab 幂等，存在则仅激活）
    openSettingsTab();
  }
});

onBeforeUnmount(() => {
  unlistenClose?.();
  disposeEvents();
});
</script>

<template>
  <div class="app">
    <div class="app-ambient" aria-hidden="true"></div>
    <AppHeader />
    <div class="app-body">
      <LoadingScreen v-if="store.booting" />
      <template v-else>
        <EditorPane />
        <RightPanel />
      </template>
    </div>
    <ConfirmDialog />
    <div v-if="store.toast" class="toast">{{ store.toast }}</div>
    <ContextMenu
      v-if="ctxMenu"
      :items="ctxMenu.items"
      :x="ctxMenu.x"
      :y="ctxMenu.y"
      @close="ctxMenu = null"
    />
  </div>
  <TooltipLayer />
</template>
