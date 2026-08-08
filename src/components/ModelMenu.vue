<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { store } from "../composables/useCodex";

const emit = defineEmits<{ close: [] }>();
const model = ref(store.model ?? "");
const effort = ref(store.effort ?? "");
const models = ref<ModelItem[]>([]);

interface ModelItem {
  id: string;
  model: string;
  displayName: string;
  description: string;
  hidden: boolean;
  isDefault: boolean;
  supportedReasoningEfforts: { reasoningEffort: string; description: string }[];
  defaultReasoningEffort: string;
}

const EFFORT_LABELS: Record<string, string> = {
  none: "无",
  minimal: "极低",
  low: "低",
  medium: "中",
  high: "高",
  xhigh: "极高",
  max: "最高",
  ultra: "极速",
};

// 跟随所选模型动态生成强度选项；未知模型回退到常用三档
const effortOptions = computed(() => {
  const m = models.value.find((x) => x.model === model.value);
  const supported = m?.supportedReasoningEfforts ?? [];
  if (supported.length) return supported;
  return [
    { reasoningEffort: "low", description: "低" },
    { reasoningEffort: "medium", description: "中" },
    { reasoningEffort: "high", description: "高" },
  ];
});

const defaultEffort = computed(() => {
  const m = models.value.find((x) => x.model === model.value);
  return m?.defaultReasoningEffort ?? "";
});

function selectModel(m: ModelItem) {
  model.value = m.model;
  // 当前强度不在该模型支持范围内时，跟随模型默认
  if (
    !effort.value ||
    !m.supportedReasoningEfforts.some((s) => s.reasoningEffort === effort.value)
  ) {
    effort.value = m.defaultReasoningEffort ?? "";
  }
}

async function loadModels() {
  try {
    const res = await invoke<{ data: ModelItem[] }>("codex_rpc", {
      method: "model/list",
      params: {},
    });
    models.value = (res.data ?? []).filter((m) => !m.hidden);
  } catch {
    // 模型列表不可用时仅保留自定义输入
  }
}

onMounted(loadModels);

function apply() {
  // 进程级生效，不写配置文件
  store.model = model.value.trim() ? model.value.trim() : null;
  store.effort = effort.value || null;
  emit("close");
}
</script>

<template>
  <div class="popup-menu right" @click.stop>
    <div class="menu-group">
      <div class="menu-group-title">模型</div>
      <div v-if="models.length" class="question-options" style="padding: 0 8px 4px">
        <button
          class="option-btn"
          :class="{ selected: model === '' }"
          @click="model = ''"
        >
          （默认）
        </button>
        <button
          v-for="m in models"
          :key="m.id"
          class="option-btn"
          :class="{ selected: model === m.model }"
          @click="selectModel(m)"
          :title="m.description || m.model"
        >
          <span>{{ m.displayName || m.model }}</span>
          <span v-if="m.model !== (m.displayName || m.model)" class="model-slug">{{
            m.model
          }}</span>
        </button>
      </div>
      <input v-model="model" class="menu-search" placeholder="模型名称（留空使用默认）" />
      <div class="menu-group-title" style="margin-top: 10px">
        推理强度
        <span v-if="defaultEffort" style="color: var(--text-faint)">
          （默认 {{ EFFORT_LABELS[defaultEffort] ?? defaultEffort }}）
        </span>
      </div>
      <div class="question-options" style="padding: 0 8px 4px">
        <button
          v-for="e in effortOptions"
          :key="e.reasoningEffort"
          class="option-btn"
          :class="{ selected: effort === e.reasoningEffort }"
          @click="effort = e.reasoningEffort"
          :title="e.description"
        >
          {{ EFFORT_LABELS[e.reasoningEffort] ?? e.reasoningEffort }}
        </button>
        <button
          class="option-btn"
          :class="{ selected: effort === '' }"
          @click="effort = ''"
        >
          默认
        </button>
      </div>
      <div class="modal-foot" style="border-top: none; padding: 8px 8px 2px">
        <button class="btn primary" @click="apply()">应用</button>
      </div>
    </div>
  </div>
</template>
