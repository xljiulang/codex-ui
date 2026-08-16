<script setup lang="ts">
import { computed, ref } from "vue";
import MarkdownText from "./MarkdownText.vue";
import { copyText } from "../lib/clipboard";
import { splitPlanTitle } from "../lib/planText";

const props = defineProps<{
  /** 计划正文（原始 Markdown 源码，含计划自带标题） */
  planText: string;
  /** 初始是否展开；默认折叠 */
  defaultOpen?: boolean;
}>();

/** 标题取自计划 Markdown 的首个标题行；正文为标题行之后内容 */
const plan = computed(() => splitPlanTitle(props.planText));
const open = ref(props.defaultOpen ?? false);
const copied = ref(false);

async function copyPlan() {
  if (!props.planText) return;
  copied.value = await copyText(props.planText);
  window.setTimeout(() => {
    copied.value = false;
  }, 1500);
}
</script>

<template>
  <div class="plan-text-card">
    <div class="plan-text-head">
      <button
        class="plan-text-toggle"
        :aria-expanded="open"
        aria-label="展开或收起计划"
        @click="open = !open"
      >
        <span class="plan-text-arrow">{{ open ? "▾" : "▸" }}</span>
        <span class="plan-text-title">{{ plan.title }}</span>
      </button>
      <button
        v-if="planText"
        class="plan-text-copy"
        :aria-label="'复制计划'"
        @click="copyPlan()"
      >
        {{ copied ? "已复制" : "复制计划" }}
      </button>
    </div>
    <div v-if="open" class="plan-text-body">
      <MarkdownText :text="plan.body" />
    </div>
  </div>
</template>
