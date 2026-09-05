<script setup lang="ts">
import {
  saveSettings,
  setToast,
  store,
  toastError,
} from "../../composables/useCodex";
import { dynamicToolRows, type DynamicToolRow } from "../../lib/dynamicTools";
import { ICON_TOOL } from "../../lib/icons";

defineProps<{ active: boolean }>();

/** 动态工具行（静态定义展开；当前为 codexui 命名空间两个工具），只提供启用/禁用 */
const dynamicToolRowsList = dynamicToolRows();

function isDynamicToolDisabled(key: string): boolean {
  return (store.settings.dynamic_tools_disabled ?? []).includes(key);
}

/** 切换动态工具启用/禁用：写入应用设置，禁用的工具不再注入新会话 */
async function toggleDynamicTool(tool: DynamicToolRow) {
  const disabled = new Set(store.settings.dynamic_tools_disabled ?? []);
  if (disabled.has(tool.key)) disabled.delete(tool.key);
  else disabled.add(tool.key);
  try {
    await saveSettings({ dynamic_tools_disabled: Array.from(disabled) });
  } catch (e) {
    setToast(toastError(e));
  }
}
</script>

<template>
  <section v-show="active" class="settings-section settings-section-dynamic-tools">
    <h2 class="settings-section-title">动态工具</h2>
    <p class="settings-section-desc">
      控制 codex-ui 动态工具（codexui）是否随新建会话注入；禁用的工具不再注入
    </p>
    <div class="model-config-card">
      <div class="model-config-card-head">
        <h3>对话内动态工具</h3>
        <div class="model-config-head-actions">
          <span class="dynamic-tools-hint">禁用后新会话不再注入</span>
        </div>
      </div>
      <div class="skills-list">
        <div
          v-for="tool in dynamicToolRowsList"
          :key="tool.key"
          class="dynamic-tool-row"
        >
          <div class="skill-info">
            <button
              type="button"
              class="skill-row-main"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_TOOL" />
              </svg>
              <span class="skill-name">{{ tool.display }}</span>
            </button>
            <p v-if="tool.description" class="skill-desc">
              {{ tool.description }}
            </p>
          </div>
          <div class="skill-actions">
            <label class="switch">
              <input
                type="checkbox"
                :checked="!isDynamicToolDisabled(tool.key)"
                :aria-label="
                  isDynamicToolDisabled(tool.key) ? '启用工具' : '禁用工具'
                "
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
