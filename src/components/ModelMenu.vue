<script setup lang="ts">
import { onMounted, ref } from "vue";
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
  supportedReasoningEfforts: { effort: string }[];
  defaultReasoningEffort: string;
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
          @click="model = m.model"
          :title="m.description || m.model"
        >
          <span>{{ m.displayName || m.model }}</span>
          <span v-if="m.model !== (m.displayName || m.model)" class="model-slug">{{
            m.model
          }}</span>
        </button>
      </div>
      <input v-model="model" class="menu-search" placeholder="模型名称（留空使用默认）" />
      <div class="menu-group-title" style="margin-top: 10px">推理强度</div>
      <div class="question-options" style="padding: 0 8px 4px">
        <button
          v-for="e in [
            { id: 'low', label: '低' },
            { id: 'medium', label: '中' },
            { id: 'high', label: '高' },
          ]"
          :key="e.id"
          class="option-btn"
          :class="{ selected: effort === e.id }"
          @click="effort = e.id"
        >
          {{ e.label }}
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
