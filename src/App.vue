<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import AppHeader from "./components/AppHeader.vue";
import ContextMenu from "./components/ContextMenu.vue";
import EditorPane from "./components/EditorPane.vue";
import RightPanel from "./components/RightPanel.vue";
import LoadingScreen from "./components/LoadingScreen.vue";
import SettingsView from "./components/SettingsView.vue";
import ConfirmDialog from "./components/ConfirmDialog.vue";
import TooltipLayer from "./components/TooltipLayer.vue";
import { disposeEvents, init, store } from "./composables/useCodex";
import { registerCloseGuard } from "./composables/useCloseGuard";
import { useContextMenu } from "./composables/useContextMenu";

const { ctxMenu } = useContextMenu();

let unlistenClose: (() => void) | undefined;

onMounted(async () => {
  unlistenClose = await registerCloseGuard();
  void init();
});

onBeforeUnmount(() => {
  unlistenClose?.();
  disposeEvents();
});
</script>

<template>
  <LoadingScreen v-if="store.booting" />
  <div v-else class="app">
    <AppHeader />
    <div class="app-body">
      <EditorPane />
      <RightPanel />
    </div>
    <SettingsView v-if="store.showSettings" />
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
