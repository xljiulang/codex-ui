<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import type { TerminalEditorTab } from "../composables/useEditorTabs";

const props = defineProps<{ tab: TerminalEditorTab }>();

const hostRef = ref<HTMLDivElement | null>(null);

interface TerminalOutputPayload {
  id: string;
  data: string;
}

interface TerminalExitPayload {
  id: string;
  exitCode: number;
}

let term: Terminal | null = null;
let fitAddon: FitAddon | null = null;
let unlistenOutput: UnlistenFn | null = null;
let unlistenExit: UnlistenFn | null = null;
let resizeObserver: ResizeObserver | null = null;
let disposed = false;

/** 把 xterm 当前尺寸同步给后端 ConPTY */
function syncSize() {
  if (!fitAddon || props.tab.exited || props.tab.error) return;
  const dims = fitAddon.proposeDimensions();
  if (!dims || dims.cols <= 0 || dims.rows <= 0) return;
  void invoke("terminal_resize", {
    id: props.tab.id,
    cols: dims.cols,
    rows: dims.rows,
  }).catch(() => {});
}

onMounted(async () => {
  const host = hostRef.value;
  if (!host) return;

  term = new Terminal({
    fontFamily: `"Cascadia Code", Consolas, "Courier New", monospace`,
    fontSize: 13,
    cursorBlink: true,
    scrollback: 5000,
    theme: {
      background: "#0c1016",
      foreground: "#d4dce8",
      cursor: "#4f8cc9",
      selectionBackground: "rgba(79, 140, 201, 0.35)",
    },
  });
  fitAddon = new FitAddon();
  term.loadAddon(fitAddon);
  term.open(host);
  fitAddon.fit();
  syncSize();

  term.onData((data) => {
    if (disposed || props.tab.exited || props.tab.error) return;
    void invoke("terminal_write", { id: props.tab.id, data }).catch(() => {});
  });

  unlistenOutput = await listen<TerminalOutputPayload>("terminal/output", (e) => {
    if (disposed || e.payload.id !== props.tab.id) return;
    term?.write(e.payload.data);
  });
  unlistenExit = await listen<TerminalExitPayload>("terminal/exit", (e) => {
    if (disposed || e.payload.id !== props.tab.id) return;
    props.tab.exited = true;
    props.tab.exitCode = e.payload.exitCode;
  });

  // 面板尺寸变化（窗口缩放/切回标签）时重新 fit 并同步 ConPTY
  if (typeof ResizeObserver !== "undefined") {
    resizeObserver = new ResizeObserver(() => {
      fitAddon?.fit();
      syncSize();
    });
    resizeObserver.observe(host);
  }

  term.focus();
});

onBeforeUnmount(() => {
  disposed = true;
  unlistenOutput?.();
  unlistenExit?.();
  resizeObserver?.disconnect();
  term?.dispose();
  term = null;
  fitAddon = null;
});
</script>

<template>
  <div class="terminal-pane">
    <div ref="hostRef" class="terminal-host"></div>
    <div v-if="tab.error" class="terminal-overlay">
      <div class="terminal-overlay-title">终端启动失败</div>
      <div class="terminal-overlay-msg">{{ tab.error }}</div>
    </div>
    <div v-else-if="tab.exited" class="terminal-overlay">
      <div class="terminal-overlay-title">
        进程已退出（代码 {{ tab.exitCode ?? "?" }}）
      </div>
      <div class="terminal-overlay-msg">可在标签上点 × 关闭该终端</div>
    </div>
  </div>
</template>
