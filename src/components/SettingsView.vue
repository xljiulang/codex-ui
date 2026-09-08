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
  "skills",
  "mcp",
  "plugins",
  "scheduled-tasks",
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
  { id: "skills", label: "技能管理", icon: ICON_SKILL },
  { id: "mcp", label: "MCP管理", icon: ICON_MCP },
  { id: "plugins", label: "插件管理", icon: ICON_EXTENSION },
  { id: "scheduled-tasks", label: "定时任务", icon: ICON_HISTORY },
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

<style scoped>
/* 设置页（设置 Tab 全宽布局，替代原模态框） */
.settings-page {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--bg);
}

.settings-page-body {
  flex: 1;
  display: flex;
  min-height: 0;
}

/* 控件与按钮同高：输入框 / 下拉 / 按钮统一 32px 高（下拉为 AppSelect 自绘组件） */
.settings-page :deep(input[type="text"]),
.settings-page :deep(input[type="password"]) {
  height: var(--ctrl-h-md);
  padding: var(--space-2) var(--space-4);
}

/* 左侧分类导航 */
.settings-nav {
  width: 176px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: var(--space-6) var(--space-4);
  border-right: 1px solid var(--border);
  overflow-y: auto;
}

.settings-nav-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: var(--space-3);
  width: 100%;
  padding: var(--space-3) var(--space-5);
  border: none;
  border-radius: var(--radius);
  background: transparent;
  color: var(--text-dim);
  font-size: var(--font-md);
  text-align: left;
  cursor: pointer;
  transition: background var(--ease), color var(--ease);
}

.settings-nav-item::before {
  content: "";
  position: absolute;
  left: 0;
  top: 50%;
  transform: translateY(-50%);
  width: 3px;
  height: 0;
  border-radius: 999px;
  background: var(--accent);
  transition: height var(--ease);
}

.settings-nav-item:hover {
  background: var(--bg-hover);
  color: var(--text-bright);
}

.settings-nav-item svg {
  width: 15px;
  height: 15px;
  fill: currentColor;
  flex-shrink: 0;
}

/* 右侧内容面板 */
.settings-panel {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow-y: auto;
  padding: var(--space-10) var(--space-12) 32px;
}
</style>
