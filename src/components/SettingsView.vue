<script setup lang="ts">
import { ref } from "vue";
import {
  ICON_EXTENSION,
  ICON_FILE,
  ICON_HISTORY,
  ICON_MCP,
  ICON_MODEL_CUBE,
  ICON_PALETTE,
  ICON_SKILL,
  ICON_TOOL,
  ICON_TUNE,
} from "../lib/icons";
import BasicSection from "./settings/BasicSection.vue";
import DynamicToolsSection from "./settings/DynamicToolsSection.vue";
import GlobalInstructionsSection from "./settings/GlobalInstructionsSection.vue";
import McpSection from "./settings/McpSection.vue";
import ModelConfigSection from "./settings/ModelConfigSection.vue";
import PersonalizationSection from "./settings/PersonalizationSection.vue";
import PluginsSection from "./settings/PluginsSection.vue";
import ScheduledTasksSection from "./settings/ScheduledTasksSection.vue";
import SkillsSection from "./settings/SkillsSection.vue";

/** 设置分类（左侧纵向导航；后续新增大类只需在此追加并补充右侧内容区） */
const settingsSectionIds = [
  "personalization",
  "basic",
  "global-instructions",
  "model-config",
  "dynamic-tools",
  "scheduled-tasks",
  "skills",
  "mcp",
  "plugins",
] as const;
type SettingsSectionId = (typeof settingsSectionIds)[number];
interface SettingsSection {
  id: SettingsSectionId;
  label: string;
  icon: string;
  /** 是否以描边渲染（当前仅「模型配置」用立方体线框） */
  stroke?: boolean;
}
const settingsSections: SettingsSection[] = [
  { id: "personalization", label: "个性化", icon: ICON_PALETTE },
  { id: "basic", label: "基础设置", icon: ICON_TUNE },
  { id: "global-instructions", label: "全局指令", icon: ICON_FILE },
  { id: "model-config", label: "模型配置", icon: ICON_MODEL_CUBE, stroke: true },
  { id: "dynamic-tools", label: "动态工具", icon: ICON_TOOL },
  { id: "scheduled-tasks", label: "定时任务", icon: ICON_HISTORY },
  { id: "skills", label: "技能管理", icon: ICON_SKILL },
  { id: "mcp", label: "MCP管理", icon: ICON_MCP },
  { id: "plugins", label: "插件管理", icon: ICON_EXTENSION },
];
/** 当前选中分类：默认取第一个分类（不依赖具体标签）；设置标签存在期间保持状态，关闭后重开才重置 */
const activeSection = ref<SettingsSectionId>(settingsSections[0].id);

/** 导航键盘操作：上下方向键循环切换分类（与其它面板方向键习惯一致） */
function onNavKeydown(e: KeyboardEvent) {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
  e.preventDefault();
  const idx = settingsSections.findIndex((s) => s.id === activeSection.value);
  const delta = e.key === "ArrowDown" ? 1 : -1;
  activeSection.value =
    settingsSections[(idx + delta + settingsSections.length) % settingsSections.length].id;
}
</script>

<template>
  <div class="settings-page">
    <div class="settings-page-body">
      <nav
        class="settings-nav"
        aria-label="设置分类"
        @keydown="onNavKeydown"
      >
        <button
          v-for="s in settingsSections"
          :key="s.id"
          class="settings-nav-item"
          :class="{ active: activeSection === s.id }"
          :aria-pressed="activeSection === s.id"
          @click="activeSection = s.id"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              :d="s.icon"
              :fill="s.stroke ? 'none' : 'currentColor'"
              :stroke="s.stroke ? 'currentColor' : 'none'"
              :stroke-width="s.stroke ? 1.5 : undefined"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
          {{ s.label }}
        </button>
      </nav>
      <div class="settings-panel">
        <PersonalizationSection :active="activeSection === 'personalization'" />

        <ModelConfigSection :active="activeSection === 'model-config'" />

        <GlobalInstructionsSection :active="activeSection === 'global-instructions'" />

        <BasicSection :active="activeSection === 'basic'" />

        <DynamicToolsSection :active="activeSection === 'dynamic-tools'" />

        <ScheduledTasksSection :active="activeSection === 'scheduled-tasks'" />

        <SkillsSection :active="activeSection === 'skills'" />

        <McpSection :active="activeSection === 'mcp'" />

        <PluginsSection :active="activeSection === 'plugins'" />

      </div>
    </div>
  </div>
</template>
