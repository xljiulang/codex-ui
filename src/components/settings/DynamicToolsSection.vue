<script setup lang="ts">
import {
  saveSettings,
  setToast,
  store,
  toastError,
} from "../../composables/useCodex";
import {
  dynamicToolRows,
  isDynamicToolEnabled,
  type DynamicToolRow,
} from "../../lib/dynamicTools";
import { ICON_TOOL } from "../../lib/icons";

defineProps<{ active: boolean }>();

/** 动态工具行（静态定义展开），只提供启用/禁用 */
const dynamicToolRowsList = dynamicToolRows();

/** 开关显示的是「生效值」：显式配置优先，缺省用工具定义里的 defaultEnabled */
function isToolOn(tool: DynamicToolRow): boolean {
  return isDynamicToolEnabled(
    tool.namespace,
    tool.tool,
    store.settings.dynamic_tools_state,
  );
}

/**
 * 切换开关：写入显式三态值（true 开启 / false 关闭），不再回落到代码缺省；
 * 只影响新建会话的注入（既有会话不变）。
 */
async function toggleDynamicTool(tool: DynamicToolRow) {
  const next = {
    ...(store.settings.dynamic_tools_state ?? {}),
    [tool.key]: !isToolOn(tool),
  };
  try {
    await saveSettings({ dynamic_tools_state: next });
  } catch (e) {
    setToast(toastError(e));
  }
}
</script>

<template>
  <section v-show="active" class="settings-section settings-section-dynamic-tools">
    <h2 class="settings-section-title">动态工具</h2>
    <p class="settings-section-desc">
      控制 codex-ui 动态工具（codexui）是否随新建会话注入；关闭的工具不再注入。知识库相关工具默认关闭
    </p>
    <div class="model-config-card">
      <div class="model-config-card-head">
        <h3>对话内动态工具</h3>
        <div class="model-config-head-actions">
          <span class="dynamic-tools-hint">改动只影响新建会话</span>
        </div>
      </div>
      <div class="skills-list">
        <div
          v-for="tool in dynamicToolRowsList"
          :key="tool.key"
          class="dynamic-tool-row"
        >
          <span class="row-icon" aria-hidden="true">
            <svg viewBox="0 0 24 24">
              <path :d="ICON_TOOL" fill="currentColor" />
            </svg>
          </span>
          <div class="skill-info">
            <span class="skill-row-main">
              <span class="skill-name">{{ tool.display }}</span>
            </span>
            <p v-if="tool.description" class="skill-desc">
              {{ tool.description }}
            </p>
          </div>
          <div class="skill-actions">
            <label class="switch">
              <input
                type="checkbox"
                :checked="isToolOn(tool)"
                :aria-label="isToolOn(tool) ? '禁用工具' : '启用工具'"
                @change="toggleDynamicTool(tool)"
              />
              <span class="switch-track"></span>
            </label>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
/* 动态工具卡片头部摘要提示 */
.dynamic-tools-hint {
  font-size: var(--font-xs);
  color: var(--text-faint);
  white-space: nowrap;
}
</style>
