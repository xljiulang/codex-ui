<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import AppHeader from "./components/AppHeader.vue";
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
