<script setup lang="ts">
import { onMounted, reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { setToast, toastError } from "../../composables/useCodex";
import { openPathInAppOrReveal } from "../../composables/usePathOpen";
import { ICON_REFRESH, ICON_SAVE } from "../../lib/icons";
import type { CustomInstructionsState } from "../../lib/types";

defineProps<{ active: boolean }>();

/** CODEX_HOME/AGENTS.md 卡片状态 */
const agents = reactive({
  loading: false,
  saving: false,
  agents_path: "",
  exists: false,
  content: "",
});

onMounted(() => {
  void loadCustomInstructions();
});

async function loadCustomInstructions() {
  agents.loading = true;
  try {
    const res = await invoke<CustomInstructionsState>(
      "custom_instructions_read",
    );
    agents.agents_path = res.agents_path;
    agents.exists = res.exists;
    agents.content = res.content;
  } catch (e) {
    setToast(toastError(e));
  } finally {
    agents.loading = false;
  }
}

async function saveCustomInstructions() {
  if (agents.saving || agents.loading) return;
  agents.saving = true;
  try {
    await invoke("custom_instructions_save", { content: agents.content });
    agents.exists = true;
    setToast("AGENTS 已保存（新会话生效）");
  } catch (e) {
    setToast(toastError(e));
  } finally {
    agents.saving = false;
  }
}

function openAgentsFile() {
  void openPathInAppOrReveal(agents.agents_path);
}
</script>

<template>
  <section
    v-show="active"
    class="settings-section settings-section-global-instructions"
  >
    <h2 class="settings-section-title">全局指令</h2>
    <p class="settings-section-desc">
      管理 CODEX_HOME 下的全局自定义指令（AGENTS.md）
    </p>
    <div class="model-config-card">
      <div class="model-config-card-head">
        <button
          type="button"
          class="model-config-title-link"
          v-tooltip="'在编辑器中打开文件'"
          :disabled="!agents.agents_path"
          @click="openAgentsFile"
        >
          <span>AGENTS</span>
        </button>
        <div class="model-config-head-actions">
          <span v-if="agents.agents_path && !agents.exists" class="model-config-missing">
            （文件不存在，保存时将新建）
          </span>
          <button
            class="btn btn-icon model-config-reload-btn"
            aria-label="刷新"
            v-tooltip="'刷新'"
            :disabled="agents.loading || agents.saving"
            @click="loadCustomInstructions"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_REFRESH" />
            </svg>
          </button>
        </div>
      </div>
      <textarea
        v-model="agents.content"
        class="custom-instructions-textarea"
        :disabled="agents.loading"
        placeholder="在此编辑 AGENTS 内容（Codex 全局自定义指令）"
        spellcheck="false"
      ></textarea>
      <div class="model-config-actions">
        <button
          class="btn primary model-config-save-btn"
          :class="{ loading: agents.saving }"
          aria-label="保存"
          :disabled="agents.saving || agents.loading"
          @click="saveCustomInstructions"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_SAVE" />
          </svg>
          <span>保存</span>
        </button>
      </div>
    </div>
  </section>
</template>
