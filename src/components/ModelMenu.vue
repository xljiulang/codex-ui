<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { loadModels, store } from "../composables/useCodex";

const emit = defineEmits<{ close: [] }>();
const model = ref(store.model ?? "");
const effort = ref(store.effort ?? "");

// 当前生效模型：显式选中优先，否则取 isDefault 模型
const effectiveModel = computed(
  () =>
    store.models.find((x) => x.model === model.value) ??
    store.models.find((x) => x.isDefault) ??
    null,
);

// 跟随生效模型动态生成强度选项；未知模型回退到常用三档
const effortOptions = computed(() => {
  const supported = effectiveModel.value?.supportedReasoningEfforts ?? [];
  if (supported.length) return supported;
  return [
    { reasoningEffort: "low", description: "低" },
    { reasoningEffort: "medium", description: "中" },
    { reasoningEffort: "high", description: "高" },
  ];
});

const defaultEffort = computed(() => {
  return effectiveModel.value?.defaultReasoningEffort ?? "";
});

function selectModel(m: (typeof store.models)[number]) {
  // 默认模型 = 服务端默认（model 置空）；其它模型显式选择
  model.value = m.isDefault ? "" : m.model;
  // 当前强度不在该模型支持范围内时，回到默认
  if (
    effort.value &&
    !m.supportedReasoningEfforts.some((s) => s.reasoningEffort === effort.value)
  ) {
    effort.value = "";
  }
}

onMounted(() => void loadModels());

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
      <div v-if="store.models.length" class="question-options" style="padding: 0 8px 4px">
        <button
          v-for="m in store.models"
          :key="m.id"
          class="option-btn"
          :class="{ selected: model === m.model || (model === '' && m.isDefault) }"
          @click="selectModel(m)"
          :title="m.description || m.model"
        >
          <span>{{ m.displayName || m.model }}</span>
          <span v-if="m.isDefault" class="model-default-tag">默认</span>
        </button>
      </div>
      <div class="menu-group-title" style="margin-top: 10px">
        推理强度
        <span v-if="defaultEffort" style="color: var(--text-faint)">（默认 {{ defaultEffort }}）</span>
      </div>
      <div class="question-options" style="padding: 0 8px 4px">
        <button
          v-for="e in effortOptions"
          :key="e.reasoningEffort"
          class="option-btn"
          :class="{
            selected:
              effort === e.reasoningEffort ||
              (effort === '' && e.reasoningEffort === defaultEffort),
          }"
          @click="effort = e.reasoningEffort === defaultEffort ? '' : e.reasoningEffort"
          :title="e.description"
        >
          {{ e.reasoningEffort }}
        </button>
      </div>
      <div class="modal-foot" style="border-top: none; padding: 8px 8px 2px">
        <button class="btn primary" @click="apply()">应用</button>
      </div>
    </div>
  </div>
</template>
