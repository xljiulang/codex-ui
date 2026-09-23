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
import {
  disposeEvents,
  init,
  openSession,
  restoreLastSession,
  store,
} from "./composables/useCodex";
import { registerCloseGuard } from "./composables/useCloseGuard";
import { useContextMenu } from "./composables/useContextMenu";
import { openSettingsTab } from "./composables/useEditorTabs";
import { sessionLog } from "./lib/sessionLog";
import { useGlobalDragDrop } from "./composables/useGlobalDragDrop";

const { ctxMenu } = useContextMenu();
const { dragging: globalDragging, setup: setupGlobalDragDrop } =
  useGlobalDragDrop();

let unlistenClose: (() => void) | undefined;
let unlistenNotification: (() => void) | undefined;

onMounted(async () => {
  // 通知「打开会话」：由 Rust 端处理 toast 点击（定时任务终态 / codex 错误）后发事件，
  // 此处聚焦窗口并打开绑定会话
  unlistenNotification = await listen<string>(
    "notification-open-session",
    async (e) => {
      const threadId = e.payload;
      if (!threadId) return;
      await sessionLog(
        "info",
        null,
        "toast-open-session",
        `frontend-received thread=${threadId}`,
      );
      // 窗口前置操作用 try/catch 隔离：即使失败也不阻断打开绑定会话（仅影响视觉前置）。
      try {
        const win = getCurrentWindow();
        await win.show();
        await win.unminimize();
        await win.setFocus();
      } catch (err) {
        await sessionLog(
          "error",
          null,
          "toast-open-session",
          `frontend-window-error=${String(err)}`,
        );
      }
      try {
        await openSession(threadId);
        await sessionLog(
          "info",
          null,
          "toast-open-session",
          "frontend-openSession-done",
        );
      } catch (err) {
        await sessionLog(
          "error",
          null,
          "toast-open-session",
          `frontend-openSession-error=${String(err)}`,
        );
      }
    },
  );
  // 关闭守卫仅阻止 Tauri 默认销毁窗口；关闭即隐藏到系统托盘由 Rust 端处理
  unlistenClose = await registerCloseGuard();
  try {
    await init();
    await setupGlobalDragDrop();
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
  <div class="app" :class="{ dragover: globalDragging }">
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

<style scoped>
.toast {
  position: fixed;
  bottom: 18px;
  left: 50%;
  transform: translateX(-50%);
  background: var(--float-bg);
  color: var(--text-bright);
  border: 1px solid var(--glass-border);
  border-radius: 999px;
  padding: var(--space-3) var(--space-8);
  font-size: var(--font-md);
  font-weight: 600;
  z-index: 200;
  max-width: 80vw;
  box-shadow: var(--shadow-md);
  animation: toast-in var(--ease-slow);
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.toast::before {
  content: "";
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
  flex-shrink: 0;
}

/* 全局拖拽悬停视觉反馈 */
.app.dragover {
  outline: 2px dashed var(--accent);
  outline-offset: -2px;
}
</style>
