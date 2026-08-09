<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { useThrottledRef } from "../composables/useThrottledRef";
import hljs from "../lib/highlight";
import { displayHref, openLink, workspaceRoot } from "../lib/links";
import { renderMarkdown } from "../lib/markdownRenderer";

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

// marked 解析在 Web Worker 离屏执行；这里只接收清洗后的 HTML。
// 流式期间以最新请求为准，丢弃乱序旧响应。
let renderSeq = 0;
watch(
  shownText,
  () => {
    const seq = ++renderSeq;
    void renderMarkdown(shownText.value).then((h) => {
      if (seq === renderSeq) applyHtml(h);
    });
  },
  { immediate: true },
);

// ---------- 增量 DOM 更新 ----------
const lastHtml = ref("");

function parseNodes(html: string): ChildNode[] {
  const tmp = document.createElement("div");
  tmp.innerHTML = html;
  return Array.from(tmp.childNodes);
}

function nodeHtml(n: ChildNode): string {
  return n.nodeType === Node.TEXT_NODE
    ? (n.textContent ?? "")
    : (n as Element).outerHTML;
}

/**
 * 增量应用渲染结果：
 * - 新 HTML 以旧 HTML 为前缀 → 直接追加尾部（新块）；
 * - 否则按顶层节点做公共前缀匹配，保留未变化节点（选区/高亮不丢），只替换尾部节点；
 * - 都不适用（结构重排）→ 整段替换兜底。
 */
function applyHtml(h: string) {
  const el = root.value;
  if (!el) {
    lastHtml.value = h;
    return;
  }
  const prev = lastHtml.value;
  if (h === prev) {
    if (!el.firstChild) el.innerHTML = h;
  } else if (prev && h.startsWith(prev)) {
    el.insertAdjacentHTML("beforeend", h.slice(prev.length));
  } else if (prev) {
    const oldNodes = parseNodes(prev);
    const newNodes = parseNodes(h);
    let common = 0;
    while (
      common < oldNodes.length &&
      common < newNodes.length &&
      nodeHtml(oldNodes[common]) === nodeHtml(newNodes[common])
    ) {
      common++;
    }
    if (common < oldNodes.length || common < newNodes.length) {
      const keep = Math.min(common, el.childNodes.length);
      while (el.childNodes.length > keep) el.removeChild(el.lastChild!);
      for (let i = common; i < newNodes.length; i++) {
        el.appendChild(newNodes[i].cloneNode(true));
      }
    }
  } else {
    el.innerHTML = h;
  }
  lastHtml.value = h;
  void nextTick(() => {
    decorateCodeBlocks();
    decorateLinks();
  });
}

onMounted(() => {
  // 兜底：首个结果可能在挂载前到达
  if (lastHtml.value && root.value && !root.value.firstChild) {
    applyHtml(lastHtml.value);
  } else {
    decorateCodeBlocks();
    decorateLinks();
  }
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
  <div ref="root" class="md"></div>
</template>
