<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { loadModels, setToast, store, toastError } from "../composables/useCodex";

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

// 跟随生效模型动态生成强度选项；无模型/无支持档位时为空，表示全部使用默认
const effortOptions = computed(() => {
  return effectiveModel.value?.supportedReasoningEfforts ?? [];
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

async function apply() {
  // 进程级生效，不写配置文件
  store.model = model.value.trim() ? model.value.trim() : null;
  store.effort = effort.value || null;
  // 有当前会话时立即同步到服务端：thread/settings/update 对后续回合即时生效
  // （model/effort 传 null 表示恢复默认，与 turn/start 的显式 null 语义一致）
  if (store.currentThreadId) {
    try {
      await invoke("codex_rpc", {
        method: "thread/settings/update",
        params: {
          threadId: store.currentThreadId,
          model: store.model,
          effort: store.effort,
        },
      });
    } catch (e) {
      setToast(toastError(e));
    }
  }
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
          v-tooltip="m.description || m.model"
        >
          <span>{{ m.displayName || m.model }}</span>
          <span v-if="m.isDefault" class="option-default-tag">默认</span>
        </button>
      </div>
      <div v-else class="menu-note">无可用模型（使用默认）</div>
      <template v-if="effortOptions.length">
        <div class="menu-group-title" style="margin-top: 10px">推理强度</div>
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
            v-tooltip="e.description"
          >
            {{ e.reasoningEffort }}
            <span v-if="e.reasoningEffort === defaultEffort" class="option-default-tag">默认</span>
          </button>
        </div>
      </template>
      <div class="modal-foot" style="border-top: none; padding: 8px 8px 2px">
        <button class="btn primary" @click="apply()">应用</button>
      </div>
    </div>
  </div>
</template>
