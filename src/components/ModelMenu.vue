<script setup lang="ts">
import { ref } from "vue";
import { store } from "../composables/useCodex";

const emit = defineEmits<{ close: [] }>();
const model = ref(store.model ?? "");
const effort = ref(store.effort ?? "");

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
