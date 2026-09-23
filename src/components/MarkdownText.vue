<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { useThrottledRef } from "../composables/useThrottledRef";
import hljs from "../lib/highlight";
import {
  displayHref,
  localPathFromHref,
  openLink,
  sessionWorkspace,
} from "../lib/links";
import { openPathInApp } from "../composables/useSessionFs";
import { renderMarkdown } from "../lib/markdownRenderer";
import { copyText } from "../lib/clipboard";
import { resolveImageSrc } from "../lib/pdfExport";
import { hideTooltip, showTooltip } from "../composables/useTooltip";

const props = defineProps<{
  text: string;
  streaming?: boolean;
  basePath?: string;
}>();
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
        decorateImages();
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
    decorateImages();
  });
}

onMounted(() => {
  // 兜底：首个结果可能在挂载前到达
  if (lastHtml.value && root.value && !root.value.firstChild) {
    applyHtml(lastHtml.value);
  } else {
    decorateCodeBlocks();
    decorateLinks();
    decorateImages();
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
      ? (/language-([\w-]+)/.exec(code.className)?.[1] ?? "")
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
  const links = root.value.querySelectorAll<HTMLAnchorElement>(
    "a:not([data-link-ready])",
  );
  for (const a of links) {
    a.setAttribute("data-link-ready", "1");
    const href = a.getAttribute("href") ?? "";
    // 悬停显示完整 URL：使用全局自定义 tooltip，而非原生 title
    const tip = displayHref(href, sessionWorkspace());
    a.addEventListener("mouseenter", () =>
      showTooltip(tip, a.getBoundingClientRect(), a),
    );
    a.addEventListener("mouseleave", () => hideTooltip());
    a.addEventListener("click", (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      const h = a.getAttribute("href") ?? "";
      const root = sessionWorkspace();
      const cls = localPathFromHref(h, root);
      // 网页/无法分类：保持原 openLink 行为（浏览器/忽略）
      if (!cls || cls.kind === "web") {
        openLink(h, root);
        return;
      }
      // 本地：支持则在应用内 tab 打开，否则降级资源管理器（原行为）
      void openPathInApp(cls.path)
        .then((opened) => {
          if (!opened) openLink(h, root);
        })
        .catch(() => openLink(h, root));
    });
  }
}

/** 相对路径本地图片（md 文件目录为基准）转 asset URL；
 *  仅当传入 basePath（文件预览）时启用，聊天气泡等无文件基准场景不处理 */
function decorateImages() {
  if (!root.value || !props.basePath) return;
  const imgs = root.value.querySelectorAll<HTMLImageElement>(
    "img:not([data-img-ready])",
  );
  for (const img of imgs) {
    img.setAttribute("data-img-ready", "1");
    const src = img.getAttribute("src");
    if (!src) continue;
    const resolved = resolveImageSrc(src, props.basePath);
    if (resolved) img.setAttribute("src", resolved);
  }
}

async function copyCode(pre: HTMLPreElement, btn: HTMLButtonElement) {
  const code = (
    pre.querySelector("code")?.textContent ??
    pre.textContent ??
    ""
  ).replace(/\n$/, "");
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

<style scoped>
.md {
  line-height: 1.65;
  user-select: text;
  overflow-wrap: anywhere;
}

.md :deep(p) {
  margin-bottom: var(--space-3);
}

.md :deep(pre) {
  background: var(--code-bg);
  color: var(--code-text);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: var(--space-4) var(--space-5);
  padding-top: 38px;
  position: relative;
  overflow-x: auto;
  font-family: var(--mono);
  font-size: var(--font-md);
  margin: var(--space-3) 0;
}

.md :deep(pre::before) {
  content: "";
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 30px;
  background: var(--code-head-tint);
  border-bottom: 1px solid var(--border);
  border-radius: var(--radius) var(--radius) 0 0;
  pointer-events: none;
}

.md :deep(pre .code-lang) {
  position: absolute;
  top: 7px;
  left: 12px;
  z-index: 2;
  font-size: var(--font-xs);
  line-height: 1;
  letter-spacing: 0.8px;
  font-weight: 600;
  color: var(--text-dim);
  background: var(--code-float-bg);
  border: 1px solid var(--border);
  border-radius: 999px;
  padding: 3px var(--space-3);
  text-transform: uppercase;
  user-select: none;
}

.md :deep(pre .code-copy-btn) {
  position: absolute;
  top: 7px;
  right: 10px;
  z-index: 2;
  font-size: var(--font-xs);
  line-height: 1;
  color: var(--text-dim);
  background: var(--code-float-bg);
  border: 1px solid var(--border-light);
  border-radius: 999px;
  padding: var(--space-1) var(--space-4);
  opacity: 0.6;
  transition:
    opacity var(--ease),
    color var(--ease),
    border-color var(--ease),
    background-color var(--ease);
  cursor: pointer;
}

.md :deep(pre:hover .code-copy-btn),
.md :deep(.code-copy-btn:hover),
.md :deep(.code-copy-btn:focus) {
  opacity: 1;
}

.md :deep(.code-copy-btn:hover) {
  color: var(--text-bright);
  border-color: var(--text-dim);
  background: var(--code-float-bg-hover);
}

.md :deep(pre code.hljs) {
  background: transparent;
  padding: 0;
}

.md :deep(code) {
  font-family: var(--mono);
  font-size: var(--font-md);
  background: rgba(var(--border-rgb), 0.12);
  border: 1px solid rgba(var(--border-rgb), 0.1);
  padding: 1px var(--space-1);
  border-radius: var(--radius-sm);
}

.md :deep(pre code) {
  background: none;
  padding: 0;
}

.md :deep(ul),
.md :deep(ol) {
  padding-left: var(--space-10);
  margin-bottom: var(--space-3);
}

.md :deep(h1),
.md :deep(h2),
.md :deep(h3) {
  color: var(--text-bright);
  margin: var(--space-5) 0 var(--space-3);
  font-weight: 700;
  line-height: 1.3;
}

.md :deep(h1) {
  font-size: 1.35em;
  padding-bottom: var(--space-2);
  border-bottom: 1px solid var(--border);
}

.md :deep(h2) {
  font-size: 1.2em;
}

.md :deep(h3) {
  font-size: 1.08em;
}

.md :deep(a) {
  color: var(--blue);
  text-decoration: none;
}

.md :deep(a:hover) {
  text-decoration: underline;
  color: var(--link-hover);
}

.md :deep(blockquote) {
  border-left: 3px solid var(--accent-dim);
  background: rgba(var(--accent-rgb), 0.05);
  border-radius: 0 var(--radius) var(--radius) 0;
  padding: var(--space-2) var(--space-5);
  color: var(--text-dim);
  margin: var(--space-3) 0;
}

.md :deep(img) {
  max-width: 100%;
  border-radius: var(--radius);
  margin: var(--space-2) 0;
}

.md :deep(input[type="checkbox"]) {
  accent-color: var(--accent);
  margin-right: var(--space-2);
  vertical-align: -2px;
}

.md :deep(table) {
  border-collapse: separate;
  border-spacing: 0;
  width: 100%;
  margin: var(--space-4) 0;
  font-size: var(--font-md);
  display: block;
  overflow-x: auto;
  border: 1px solid var(--border-light);
  border-radius: var(--radius);
}

.md :deep(th),
.md :deep(td) {
  border-right: 1px solid var(--border);
  border-bottom: 1px solid var(--border);
  padding: var(--space-3) 11px;
  text-align: left;
  white-space: nowrap;
}

.md :deep(th:last-child),
.md :deep(td:last-child) {
  border-right: none;
}

.md :deep(tr:last-child td) {
  border-bottom: none;
}

.md :deep(th) {
  background: var(--table-head-bg);
  color: var(--text-bright);
  font-weight: 600;
  position: sticky;
  top: 0;
}

.md :deep(tr:nth-child(even) td) {
  background: rgba(var(--overlay-rgb), 0.015);
}

.md :deep(tr:hover td) {
  background: rgba(var(--accent-rgb), 0.05);
}

.md :deep(td code),
.md :deep(th code) {
  background: rgba(var(--accent-rgb), 0.14);
  color: var(--link-hover);
  border: 1px solid rgba(var(--accent-rgb), 0.18);
  padding: 1px var(--space-1);
  border-radius: var(--radius-sm);
  font-family: var(--mono);
  font-size: var(--font-sm);
}

/* highlight.js 暗色配色（对齐现有 VS Code 风格变量） */
.md :deep(.hljs-comment),
.md :deep(.hljs-quote) {
  color: var(--syntax-comment);
}
.md :deep(.hljs-keyword),
.md :deep(.hljs-selector-tag),
.md :deep(.hljs-literal) {
  color: var(--syntax-keyword);
}
.md :deep(.hljs-string),
.md :deep(.hljs-regexp),
.md :deep(.hljs-addition) {
  color: var(--syntax-string);
}
.md :deep(.hljs-number),
.md :deep(.hljs-meta) {
  color: var(--syntax-number);
}
.md :deep(.hljs-title),
.md :deep(.hljs-section) {
  color: var(--syntax-function);
}
.md :deep(.hljs-attr),
.md :deep(.hljs-attribute),
.md :deep(.hljs-variable),
.md :deep(.hljs-template-variable),
.md :deep(.hljs-type),
.md :deep(.hljs-class .hljs-title) {
  color: var(--syntax-type);
}
.md :deep(.hljs-built_in),
.md :deep(.hljs-builtin-name) {
  color: var(--syntax-const);
}
.md :deep(.hljs-symbol),
.md :deep(.hljs-bullet) {
  color: var(--syntax-symbol);
}
.md :deep(.hljs-deletion) {
  color: var(--syntax-deletion);
}
.md :deep(.hljs-emphasis) {
  font-style: italic;
}
.md :deep(.hljs-strong) {
  font-weight: 600;
}
</style>
