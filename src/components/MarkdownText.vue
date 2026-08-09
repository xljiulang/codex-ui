<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { useThrottledRef } from "../composables/useThrottledRef";
import hljs from "../lib/highlight";
import { displayHref, openLink, workspaceRoot } from "../lib/links";

const props = defineProps<{ text: string; streaming?: boolean }>();
const root = ref<HTMLElement | null>(null);

// 流式期间最多每 80ms 刷新一次展示文本，避免每个 delta 全量重解析；
// streaming=false 时立即刷净。
const source = computed(() => props.text);
const { ref: shownText, flush: flushText } = useThrottledRef(source, 80);
watch(
  () => props.streaming,
  (s) => {
    if (!s) flushText();
  },
);

const html = computed(() => {
  const raw = marked.parse(shownText.value, {
    async: false,
    breaks: true,
    gfm: true,
  }) as string;
  // 包一层 div 再 sanitize：DOMPurify 会把作为文档根元素的 <pre> 剥离
  // （消息以代码块开头时代码块会失去 pre/复制按钮），包一层后根元素为 div，pre 正常保留。
  // 同时放行 file: 协议（默认白名单不含 file:，会导致本地文件链接 href 被剥掉）。
  return DOMPurify.sanitize(`<div>${raw}</div>`, {
    ALLOWED_URI_REGEXP:
      /^(?:(?:https?|file|mailto|tel|callto|sms|cid|xmpp|matrix):|[a-z]:(?:[\\/]|%5[cC]|%2[fF])|[^a-z]|[a-z+.\-]+(?:[^a-z+.\-:]|$))/i,
  });
});

watch(
  html,
  () => {
    void nextTick(() => {
      decorateCodeBlocks();
      decorateLinks();
    });
  },
  { immediate: true },
);

onMounted(() => {
  decorateCodeBlocks();
  decorateLinks();
});

/** 代码块：语法高亮 + 语言徽标 + 复制按钮 */
function decorateCodeBlocks() {
  if (!root.value) return;
  const pres = root.value.querySelectorAll<HTMLPreElement>(
    "pre:not([data-decorated])",
  );
  for (const pre of pres) {
    pre.setAttribute("data-decorated", "1");
    const code = pre.querySelector("code");
    const lang = code
      ? /language-([\w-]+)/.exec(code.className)?.[1] ?? ""
      : "";
    if (code && lang && !code.dataset.highlighted) {
      try {
        hljs.highlightElement(code);
        code.dataset.highlighted = "1";
      } catch {
        // 高亮失败不影响展示
      }
    }
    const chip = document.createElement("span");
    chip.className = "code-lang";
    chip.textContent = lang || "code";
    pre.appendChild(chip);
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "code-copy-btn";
    btn.textContent = "复制";
    btn.setAttribute("aria-label", "复制代码");
    btn.addEventListener("click", (ev) => {
      ev.stopPropagation();
      void copyCode(pre, btn);
    });
    pre.appendChild(btn);
  }
}

function decorateLinks() {
  if (!root.value) return;
  const links = root.value.querySelectorAll<HTMLAnchorElement>("a:not([data-link-ready])");
  for (const a of links) {
    a.setAttribute("data-link-ready", "1");
    const href = a.getAttribute("href") ?? "";
    if (!a.getAttribute("title")) {
      a.setAttribute("title", displayHref(href, workspaceRoot()));
    }
    a.addEventListener("click", (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      const h = a.getAttribute("href") ?? "";
      openLink(h, workspaceRoot());
    });
  }
}

async function copyCode(pre: HTMLPreElement, btn: HTMLButtonElement) {
  const code = (pre.querySelector("code")?.textContent ?? pre.textContent ?? "").replace(/\n$/, "");
  let ok = false;
  try {
    await navigator.clipboard.writeText(code);
    ok = true;
  } catch {
    try {
      const ta = document.createElement("textarea");
      ta.value = code;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      ok = document.execCommand("copy");
      document.body.removeChild(ta);
    } catch {
      ok = false;
    }
  }
  btn.textContent = ok ? "已复制" : "复制失败";
  window.setTimeout(() => {
    btn.textContent = "复制";
  }, 1500);
}
</script>

<template>
  <div ref="root" class="md" v-html="html"></div>
</template>
