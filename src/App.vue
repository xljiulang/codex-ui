<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import AppHeader from "./components/AppHeader.vue";
import ContextMenu from "./components/ContextMenu.vue";
import EditorPane from "./components/EditorPane.vue";
import RightPanel from "./components/RightPanel.vue";
import LoadingScreen from "./components/LoadingScreen.vue";
import ConfirmDialog from "./components/ConfirmDialog.vue";
import TooltipLayer from "./components/TooltipLayer.vue";
import { disposeEvents, init, openSession, restoreLastSession, store } from "./composables/useCodex";
import { registerCloseGuard } from "./composables/useCloseGuard";
import { useContextMenu } from "./composables/useContextMenu";
import { openSettingsTab } from "./composables/useEditorTabs";

const { ctxMenu } = useContextMenu();

let unlistenClose: (() => void) | undefined;
let unlistenNotification: (() => void) | undefined;

onMounted(async () => {
  // 定时任务通知「打开会话」：由 Rust 端处理 toast 点击后发事件，此处聚焦窗口并打开绑定会话
  unlistenNotification = await listen<string>("scheduled-task-notification-open", async (e) => {
    const threadId = e.payload;
    if (!threadId) return;
    const win = getCurrentWindow();
    await win.show();
    await win.unminimize();
    await win.setFocus();
    await openSession(threadId);
  });
  // 关闭守卫仅阻止 Tauri 默认销毁窗口；关闭即隐藏到系统托盘由 Rust 端处理
  unlistenClose = await registerCloseGuard();
  try {
  await init();
  } finally {
    // 启动恢复：配置文件记录了最后活跃会话（且线程仍在历史中）则打开该会话并展开
    // 其目录分组；否则回退打开设置标签（设置 tab 幂等，存在则仅激活）
    if (!(await restoreLastSession())) openSettingsTab();
  }
});

onBeforeUnmount(() => {
  unlistenNotification?.();
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
        <RightPanel v-show="!store.rightPanelHidden" />
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
