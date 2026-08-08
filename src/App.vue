<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import AppHeader from "./components/AppHeader.vue";
import HistoryPanel from "./components/HistoryPanel.vue";
import ChatView from "./components/ChatView.vue";
import SettingsView from "./components/SettingsView.vue";
import InteractionDialog from "./components/InteractionDialog.vue";
import GoalDialog from "./components/GoalDialog.vue";
import { disposeEvents, init, store } from "./composables/useCodex";

onMounted(() => {
  void init();
});

onBeforeUnmount(() => {
  disposeEvents();
});
</script>

<template>
  <div class="app">
    <AppHeader />
    <div class="app-body">
      <HistoryPanel v-if="store.showHistory" />
      <SettingsView v-if="store.showSettings" />
      <ChatView v-else />
    </div>
    <InteractionDialog />
    <GoalDialog v-if="store.goalOpen" />
    <div v-if="store.toast" class="toast">{{ store.toast }}</div>
  </div>
</template>
