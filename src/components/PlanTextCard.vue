<script setup lang="ts">
import { computed, ref } from "vue";
import MarkdownText from "./MarkdownText.vue";
import { copyText } from "../lib/clipboard";
import { splitPlanTitle } from "../lib/planText";
import { ICON_ARROW_DOWN, ICON_ARROW_RIGHT, ICON_PLAN } from "../lib/icons";

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
  <div class="assistant-card" :class="{ expanded: open }">
    <div class="assistant-card-header">
      <button
        type="button"
        class="assistant-card-toggle"
        :aria-expanded="open"
        aria-label="展开或收起计划"
        @click="open = !open"
      >
        <svg class="assistant-card-arrow" viewBox="0 0 24 24" aria-hidden="true">
          <path :d="open ? ICON_ARROW_DOWN : ICON_ARROW_RIGHT" />
        </svg>
        <svg class="assistant-card-icon" viewBox="0 0 24 24" aria-hidden="true">
          <path :d="ICON_PLAN" />
        </svg>
        <span class="assistant-card-title">{{ plan.title }}</span>
      </button>
      <button
        v-if="planText"
        type="button"
        class="copy-btn"
        :aria-label="'复制'"
        @click="copyPlan()"
      >
        {{ copied ? "已复制" : "复制" }}
      </button>
    </div>
    <Transition name="assistant-card-body">
      <div v-if="open" class="assistant-card-body">
        <MarkdownText :text="plan.body" />
      </div>
    </Transition>
  </div>
</template>
