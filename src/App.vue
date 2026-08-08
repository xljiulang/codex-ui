<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import AppHeader from "./components/AppHeader.vue";
import HistoryPanel from "./components/HistoryPanel.vue";
import ChatView from "./components/ChatView.vue";
import SettingsView from "./components/SettingsView.vue";
import InteractionDialog from "./components/InteractionDialog.vue";
import GoalDialog from "./components/GoalDialog.vue";
import { disposeEvents, init, store } from "./composables/useCodex";

interface CtxItem {
  label: string;
  action: () => void;
}

const ctxMenu = ref<{ x: number; y: number; items: CtxItem[] } | null>(null);

async function copyText(t: string) {
  try {
    await navigator.clipboard.writeText(t);
  } catch {
    // 剪贴板 API 不可用时回退
    try {
      const ta = document.createElement("textarea");
      ta.value = t;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      document.body.removeChild(ta);
    } catch {
      // ignore
    }
  }
}

function onContextMenu(e: MouseEvent) {
  e.preventDefault(); // 屏蔽默认菜单（含“刷新”），避免页面重载
  const target = e.target as HTMLElement;
  const editable =
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target.isContentEditable;
  const selection = window.getSelection()?.toString() ?? "";
  const link = target.closest("a") as HTMLAnchorElement | null;
  const items: CtxItem[] = [];

  if (editable) {
    items.push(
      { label: "剪切", action: () => document.execCommand("cut") },
      { label: "复制", action: () => document.execCommand("copy") },
      { label: "粘贴", action: () => document.execCommand("paste") },
      { label: "全选", action: () => document.execCommand("selectAll") },
    );
  } else if (selection) {
    items.push({ label: "复制", action: () => void copyText(selection) });
  }
  if (link) {
    items.push({
      label: "打开链接",
      action: () => void invoke("open_url", { url: link.href }),
    });
    items.push({
      label: "复制链接地址",
      action: () => void copyText(link.href),
    });
  }

  if (!items.length) {
    ctxMenu.value = null;
    return;
  }
  const x = Math.min(e.clientX, window.innerWidth - 160);
  const y = Math.min(e.clientY, window.innerHeight - items.length * 30 - 12);
  ctxMenu.value = { x, y, items };
}

function onGlobalClick() {
  ctxMenu.value = null;
}

function onGlobalKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") ctxMenu.value = null;
}

function onGlobalScroll() {
  ctxMenu.value = null;
}

onMounted(() => {
  void init();
  window.addEventListener("contextmenu", onContextMenu);
  window.addEventListener("click", onGlobalClick);
  window.addEventListener("keydown", onGlobalKeydown);
  window.addEventListener("scroll", onGlobalScroll, true);
});

onBeforeUnmount(() => {
  disposeEvents();
  window.removeEventListener("contextmenu", onContextMenu);
  window.removeEventListener("click", onGlobalClick);
  window.removeEventListener("keydown", onGlobalKeydown);
  window.removeEventListener("scroll", onGlobalScroll, true);
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
