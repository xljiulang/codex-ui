<script setup lang="ts">
import { computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { WebSearchItem } from "../lib/types";

const props = defineProps<{ item: WebSearchItem }>();

/** webSearch 的结构化结果（协议为透明 JSON，尽力提取常见字段） */
const webResults = computed(() => {
  const r = props.item.results as unknown[] | null | undefined;
  if (!Array.isArray(r)) return [];
  return r.map((x) => {
    if (x && typeof x === "object") {
      const o = x as Record<string, unknown>;
      return {
        title: String(o.title ?? o.name ?? ""),
        url: String(o.url ?? o.link ?? ""),
        snippet: String(o.snippet ?? o.description ?? o.content ?? ""),
      };
    }
    return { title: String(x), url: "", snippet: "" };
  });
});

const webResultsUsable = computed(() =>
  webResults.value.some((r) => r.title || r.url),
);
</script>

<template>
  <div class="tool-meta">查询：{{ item.query }}</div>
  <div v-if="webResultsUsable" class="web-results">
    <a
      v-for="(r, i) in webResults"
      :key="i"
      class="web-result"
      :href="r.url || '#'"
      @click.prevent="
        r.url ? void invoke('open_url', { url: r.url }) : undefined
      "
    >
      <span class="web-result-title">{{
        r.title || r.url || "（无标题）"
      }}</span>
      <span v-if="r.snippet" class="web-result-snippet">{{ r.snippet }}</span>
    </a>
  </div>
  <div v-else-if="webResults.length" class="tool-json">
    {{ JSON.stringify(item.results) }}
  </div>
</template>
