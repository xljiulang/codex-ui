<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import type { TerminalEditorTab } from "../composables/useEditorTabs";
import {
  attachTerminal,
  type TerminalHandle,
} from "../composables/useTerminalEvents";

const props = defineProps<{ tab: TerminalEditorTab; active?: boolean }>();

const hostRef = ref<HTMLDivElement | null>(null);

/** 后端 prompt 注入的空闲标记：cmd/PowerShell 每次回到提示符先输出 OSC 133;D */
const PROMPT_MARKER = "\x1b]133;D";
/** 标记前缀长度：跨输出块拆分时保留上一块尾部做拼接匹配 */
const MARKER_PREFIX_LEN = PROMPT_MARKER.length;
let tailCarry = "";

/**
 * 扫描输出块中的提示符标记：收到标记说明命令已执行完、回到空闲提示符。
 * 标记可能被分块拆分，需与上一块尾部拼接后匹配。
 */
function scanPromptMarker(chunk: string): boolean {
  const combined = tailCarry + chunk;
  const found = combined.includes(PROMPT_MARKER);
  tailCarry = found ? "" : combined.slice(-(MARKER_PREFIX_LEN - 1));
  return found;
}

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
  cursorAccent: string;
  selectionBackground: string;
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
}

/** 从当前主题 CSS 变量读取 xterm 完整配色（变量缺失时回退到蓝夜默认值） */
function readTerminalTheme(): TerminalTheme {
  const cs = getComputedStyle(document.documentElement);
  const accentRgb = cs.getPropertyValue("--accent-rgb").trim();
  const v = (name: string, fallback: string) =>
    cs.getPropertyValue(name).trim() || fallback;
  return {
    background: v("--console-bg-deep", "#0c1016"),
    foreground: v("--console-text", "#d4dce8"),
    cursor: v("--accent", "#4f8cc9"),
    cursorAccent: v("--console-cursor-text", "#0c1016"),
    selectionBackground: accentRgb
      ? `rgba(${accentRgb}, 0.35)`
      : "rgba(79, 140, 201, 0.35)",
    black: v("--console-ansi-black", "#0e1116"),
    red: v("--console-ansi-red", "#f0565f"),
    green: v("--console-ansi-green", "#46d5a8"),
    yellow: v("--console-ansi-yellow", "#e5d6a0"),
    blue: v("--console-ansi-blue", "#79b8ff"),
    magenta: v("--console-ansi-magenta", "#c792ea"),
    cyan: v("--console-ansi-cyan", "#56b6c2"),
    white: v("--console-ansi-white", "#c6cdd8"),
    brightBlack: v("--console-ansi-bright-black", "#76819a"),
    brightRed: v("--console-ansi-bright-red", "#ff7a83"),
    brightGreen: v("--console-ansi-bright-green", "#7cebc2"),
    brightYellow: v("--console-ansi-bright-yellow", "#f2e3b0"),
    brightBlue: v("--console-ansi-bright-blue", "#9ccbff"),
    brightMagenta: v("--console-ansi-bright-magenta", "#e0a7f5"),
    brightCyan: v("--console-ansi-bright-cyan", "#86d7e0"),
    brightWhite: v("--console-ansi-bright-white", "#eef2f8"),
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
    // 回车/粘贴换行视为“开始执行命令”：亮起呼吸灯，直到下一个提示符标记
    if (data.includes("\r") || data.includes("\n")) {
      props.tab.busy = true;
    }
    void invoke("terminal_write", { id: props.tab.id, data }).catch(() => {});
  });

  // 挂载前 openTerminalTab 已通过事件桥缓冲启动输出（含 ConPTY DSR 查询，
  // xterm 写入后会自动应答，提示符才能渲染），这里先回放再转实时。
  tailCarry = "";
  handle = attachTerminal(props.tab.id);
  for (const chunk of handle.flush()) {
    if (disposed) break;
    if (scanPromptMarker(chunk)) props.tab.busy = false;
    term?.write(chunk);
  }
  const bufferedExit = handle.exitCode;
  if (bufferedExit !== null && !disposed) {
    props.tab.exited = true;
    props.tab.exitCode = bufferedExit;
    props.tab.busy = false;
  }
  handle.onData((data) => {
    if (disposed) return;
    if (scanPromptMarker(data)) props.tab.busy = false;
    term?.write(data);
  });
  handle.onExit((exitCode) => {
    if (disposed) return;
    props.tab.exited = true;
    props.tab.exitCode = exitCode;
    props.tab.busy = false;
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

  if (props.active && !props.tab.exited && !props.tab.error) {
    term.focus();
  }
});

// 标签被激活（v-show 由隐藏转显示）后聚焦 xterm，切回终端即可直接输入
watch(
  () => props.active,
  (active) => {
    if (!active || disposed || props.tab.exited || props.tab.error) return;
    void nextTick(() => {
      if (disposed || props.tab.exited || props.tab.error) return;
      term?.focus();
    });
  },
);

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
