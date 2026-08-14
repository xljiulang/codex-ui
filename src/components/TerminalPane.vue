<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import type { TerminalEditorTab } from "../composables/useEditorTabs";
import {
  attachTerminal,
  type TerminalHandle,
} from "../composables/useTerminalEvents";

const props = defineProps<{ tab: TerminalEditorTab }>();

const hostRef = ref<HTMLDivElement | null>(null);

let term: Terminal | null = null;
let fitAddon: FitAddon | null = null;
let resizeObserver: ResizeObserver | null = null;
let handle: TerminalHandle | null = null;
let themeObserver: MutationObserver | null = null;
let disposed = false;

interface TerminalTheme {
  background: string;
  foreground: string;
  cursor: string;
  selectionBackground: string;
}

/** 从当前主题 CSS 变量读取 xterm 基础配色（变量缺失时回退到原有深色默认值） */
function readTerminalTheme(): TerminalTheme {
  const cs = getComputedStyle(document.documentElement);
  const accentRgb = cs.getPropertyValue("--accent-rgb").trim();
  return {
    background: cs.getPropertyValue("--console-bg-deep").trim() || "#0c1016",
    foreground: cs.getPropertyValue("--console-text").trim() || "#d4dce8",
    cursor: cs.getPropertyValue("--accent").trim() || "#4f8cc9",
    selectionBackground: accentRgb
      ? `rgba(${accentRgb}, 0.35)`
      : "rgba(79, 140, 201, 0.35)",
  };
}

/** 把当前主题配色同步给 xterm（构造后/主题切换时调用） */
function applyThemeToTerminal() {
  if (!term) return;
  term.options.theme = readTerminalTheme();
}

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

onMounted(() => {
  const host = hostRef.value;
  if (!host) return;

  term = new Terminal({
    fontFamily: `"Cascadia Code", Consolas, "Courier New", monospace`,
    fontSize: 13,
    cursorBlink: true,
    scrollback: 5000,
    allowTransparency: true,
    theme: readTerminalTheme(),
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

  // 挂载前 openTerminalTab 已通过事件桥缓冲启动输出（含 ConPTY DSR 查询，
  // xterm 写入后会自动应答，提示符才能渲染），这里先回放再转实时。
  handle = attachTerminal(props.tab.id);
  for (const chunk of handle.flush()) {
    if (disposed) break;
    term?.write(chunk);
  }
  const bufferedExit = handle.exitCode;
  if (bufferedExit !== null && !disposed) {
    props.tab.exited = true;
    props.tab.exitCode = bufferedExit;
  }
  handle.onData((data) => {
    if (disposed) return;
    term?.write(data);
  });
  handle.onExit((exitCode) => {
    if (disposed) return;
    props.tab.exited = true;
    props.tab.exitCode = exitCode;
  });

  // 面板尺寸变化（窗口缩放/切回标签）时重新 fit 并同步 ConPTY
  if (typeof ResizeObserver !== "undefined") {
    resizeObserver = new ResizeObserver(() => {
      fitAddon?.fit();
      syncSize();
    });
    resizeObserver.observe(host);
  }

  // 主题切换（设置应用与预览都改 <html data-theme>）时同步 xterm 配色
  if (typeof MutationObserver !== "undefined") {
    themeObserver = new MutationObserver(applyThemeToTerminal);
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
  }

  term.focus();
});

// spawn 完成（loading 置 false）后补一次 fit/resize，消除挂载时 resize
// 早于 spawn 完成、被后端“终端不存在”拒绝的竞态。
watch(
  () => props.tab.loading,
  (loading) => {
    if (loading || disposed || props.tab.exited || props.tab.error) return;
    fitAddon?.fit();
    syncSize();
  },
);

onBeforeUnmount(() => {
  disposed = true;
  handle?.detach();
  handle = null;
  themeObserver?.disconnect();
  themeObserver = null;
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
