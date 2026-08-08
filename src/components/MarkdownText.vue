<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { invoke } from "@tauri-apps/api/core";

const props = defineProps<{ text: string }>();
const root = ref<HTMLElement | null>(null);

const html = computed(() => {
  const raw = marked.parse(props.text, { async: false, breaks: true, gfm: true }) as string;
  return DOMPurify.sanitize(raw);
});

watch(
  html,
  () => {
    void nextTick(() => {
      injectCopyButtons();
      decorateLinks();
    });
  },
  { immediate: true },
);

onMounted(() => {
  injectCopyButtons();
  decorateLinks();
});

function injectCopyButtons() {
  if (!root.value) return;
  const pres = root.value.querySelectorAll<HTMLPreElement>("pre:not([data-copy-ready])");
  for (const pre of pres) {
    pre.setAttribute("data-copy-ready", "1");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "code-copy-btn";
    btn.textContent = "复制";
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
      a.setAttribute("title", href);
    }
    a.addEventListener("click", (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      const h = a.getAttribute("href") ?? "";
      if (/^(https?|file):\/\//i.test(h)) {
        void invoke("open_url", { url: h }).catch(() => undefined);
      }
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
