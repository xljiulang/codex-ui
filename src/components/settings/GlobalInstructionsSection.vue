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

<style scoped>
/* 「全局指令」分区撑满右侧面板剩余高度，AGENTS 卡片与编辑器随内容填充 */
.settings-section-global-instructions {
  flex: 1;
  min-height: 0;
}

/* 卡片标题作为文件链接（AGENTS / model_catalog_json）：纯文字，可点击打开文件 */
.model-config-title-link {
  display: inline-flex;
  align-items: center;
  margin: 0;
  padding: 0;
  border: none;
  background: none;
  font: inherit;
  font-size: var(--font-base);
  font-weight: 700;
  line-height: 1.2;
  color: var(--text-bright);
  cursor: pointer;
  flex-shrink: 0;
}

.model-config-title-link:hover {
  color: var(--accent);
}

.model-config-title-link:disabled {
  cursor: default;
  opacity: 0.55;
  text-decoration: none;
}

.model-config-missing {
  color: var(--red);
}

.custom-instructions-textarea {
  width: 100%;
  flex: 1;
  min-height: 240px;
  overflow-y: auto;
  resize: none;
  box-sizing: border-box;
  font-family: var(--mono);
  font-size: var(--font-md);
  line-height: 1.5;
  color: var(--text);
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: var(--space-3) var(--space-4);
}

.custom-instructions-textarea:focus {
  outline: none;
  border-color: var(--accent);
}
</style>

