<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { useThrottledRef } from "../composables/useThrottledRef";
import hljs from "../lib/highlight";
import {
  displayHref,
  localPathFromHref,
  openLink,
  workspaceRoot,
} from "../lib/links";
import { openPathInApp } from "../composables/useSessionFs";
import { renderMarkdown } from "../lib/markdownRenderer";
import { copyText } from "../lib/clipboard";
import {
  hideTooltip,
  showTooltip,
} from "../composables/useTooltip";

const props = defineProps<{ text: string; streaming?: boolean }>();
const root = ref<HTMLElement | null>(null);

// 流式期间最多每 80ms 刷新一次展示文本，避免每个 delta 全量重解析；
// streaming=false 时立即刷净。
const source = computed(() => props.text);
const { ref: shownText, flush: flushText } = useThrottledRef(source, 80);
watch(
  () => props.streaming,
  (s) => {
    if (!s) {
      flushText();
      // 结束后补齐装饰：流式期间跳过的代码块高亮在此一次性完成
      void nextTick(() => {
        decorateCodeBlocks();
        decorateLinks();
      });
    }
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
// 顶层节点的 HTML 缓存：公共前缀比较时避免每次把整段旧 HTML 重新 parse + 字符串化
let renderedNodeHtml: string[] = [];

function parseNodes(html: string): ChildNode[] {
  const tmp = document.createElement("div");
  tmp.innerHTML = html;
  return Array.from(tmp.childNodes);
}

function nodeHtml(n: Node): string {
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
    if (!el.firstChild) {
      el.innerHTML = h;
      renderedNodeHtml = parseNodes(h).map(nodeHtml);
    }
  } else if (prev && h.startsWith(prev)) {
    const added = parseNodes(h.slice(prev.length));
    for (const n of added) {
      el.appendChild(n);
      renderedNodeHtml.push(nodeHtml(n));
    }
  } else if (prev) {
    const newNodes = parseNodes(h);
    let common = 0;
    while (
      common < renderedNodeHtml.length &&
      common < newNodes.length &&
      renderedNodeHtml[common] === nodeHtml(newNodes[common])
    ) {
      common++;
    }
    if (common < renderedNodeHtml.length || common < newNodes.length) {
      const keep = Math.min(common, el.childNodes.length);
      while (el.childNodes.length > keep) el.removeChild(el.lastChild!);
      renderedNodeHtml.length = keep;
      for (let i = common; i < newNodes.length; i++) {
        const n = newNodes[i].cloneNode(true);
        el.appendChild(n);
        renderedNodeHtml.push(nodeHtml(n));
      }
    }
  } else {
    el.innerHTML = h;
    renderedNodeHtml = parseNodes(h).map(nodeHtml);
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

/** 代码块：语言徽标 + 复制按钮；语法高亮仅在非流式时执行，
 *  避免流式期间对增长中的代码块每 tick 全量重高亮（O(n²)） */
function decorateCodeBlocks() {
  if (!root.value) return;
  const streamingNow = props.streaming === true;
  const pres = root.value.querySelectorAll<HTMLPreElement>(
    streamingNow ? "pre:not([data-decorated])" : "pre:not([data-highlighted])",
  );
  for (const pre of pres) {
    const code = pre.querySelector("code");
    const lang = code
      ? /language-([\w-]+)/.exec(code.className)?.[1] ?? ""
      : "";
    if (!streamingNow && code && lang && !code.dataset.highlighted) {
      try {
        hljs.highlightElement(code);
        code.dataset.highlighted = "1";
      } catch {
        // 高亮失败不影响展示
      }
    }
    if (pre.dataset.decorated) continue;
    pre.setAttribute("data-decorated", "1");
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
    // 悬停显示完整 URL：使用全局自定义 tooltip，而非原生 title
    const tip = displayHref(href, workspaceRoot());
    a.addEventListener("mouseenter", () => showTooltip(tip, a.getBoundingClientRect()));
    a.addEventListener("mouseleave", () => hideTooltip());
    a.addEventListener("click", (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      const h = a.getAttribute("href") ?? "";
      const root = workspaceRoot();
      const cls = localPathFromHref(h, root);
      // 网页/无法分类：保持原 openLink 行为（浏览器/忽略）
      if (!cls || cls.kind === "web") {
        openLink(h, root);
        return;
      }
      // 本地：支持则在应用内 tab 打开，否则降级资源管理器（原行为）
      void openPathInApp(cls.path).then((opened) => {
        if (!opened) openLink(h, root);
      });
    });
  }
}

async function copyCode(pre: HTMLPreElement, btn: HTMLButtonElement) {
  const code = (pre.querySelector("code")?.textContent ?? pre.textContent ?? "").replace(/\n$/, "");
  const ok = await copyText(code);
  btn.textContent = ok ? "已复制" : "复制失败";
  window.setTimeout(() => {
    btn.textContent = "复制";
  }, 1500);
}
</script>

<template>
  <div ref="root" class="md"></div>
</template>
