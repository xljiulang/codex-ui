<script setup lang="ts">
import { onMounted, reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { askConfirm, setToast, toastError } from "../../composables/useCodex";
import { openPathInAppOrReveal } from "../../composables/usePathOpen";
import { openDocsUrl } from "../../lib/links";
import {
  ICON_DELETE,
  ICON_LINK,
  ICON_PLUS,
  ICON_REFRESH,
  ICON_SKILL,
} from "../../lib/icons";
import type {
  SkillErrorInfo,
  SkillsItem,
  SkillsState,
} from "../../lib/types";

/** Skills Catalog for Codex（浏览器打开） */
const SKILLS_CATALOG_URL = "https://github.com/openai/skills";

const props = defineProps<{ active: boolean }>();

const skillsState = reactive({
  loading: false,
  adding: false,
  items: [] as SkillsItem[],
  errors: [] as SkillErrorInfo[],
  busy: {} as Record<string, boolean>,
});

onMounted(() => {
  void loadSkills();
});

/** 打开文件对话框选择 SKILL.md：校验通过后安装到 CODEX_HOME/skills 并刷新列表 */
async function addSkill() {
  if (skillsState.loading || skillsState.adding) return;
  skillsState.adding = true;
  try {
    const name = await invoke<string | null>("skills_add");
    if (!name) return; // 用户取消：文件未变化，不刷新
    setToast(`已添加技能 ${name}`);
    await loadSkills(true);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    skillsState.adding = false;
  }
}

/** 拉取本地技能列表（skills_read：从 skills/list 过滤出 CODEX_HOME/skills 下的技能） */
async function loadSkills(forceReload = false) {
  if (skillsState.loading) return;
  skillsState.loading = true;
  try {
    const res = await invoke<SkillsState | null>("skills_read", {
      forceReload,
    });
    skillsState.items = res?.items ?? [];
    skillsState.errors = res?.errors ?? [];
  } catch (e) {
    setToast(toastError(e));
  } finally {
    skillsState.loading = false;
  }
}

function openSkill(s: SkillsItem) {
  void openPathInAppOrReveal(s.path);
}

/** 启用/禁用技能：写用户级技能配置后强制重读列表 */
async function toggleSkill(s: SkillsItem) {
  if (skillsState.loading || skillsState.busy[s.path]) return;
  skillsState.busy[s.path] = true;
  const next = !s.enabled;
  try {
    await invoke("codex_rpc", {
      method: "skills/config/write",
      params: { name: s.name, enabled: next },
    });
    setToast(next ? `已启用 ${s.name}` : `已禁用 ${s.name}`);
    await loadSkills(true);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    skillsState.busy[s.path] = false;
  }
}

/** 删除技能：确认后删除 SKILL.md 所在目录并强制刷新列表 */
async function removeSkill(s: SkillsItem) {
  if (skillsState.loading || skillsState.busy[s.path]) return;
  const ok = await askConfirm({
    title: "删除技能",
    message: `确定删除技能「${s.name}」吗？将删除该技能所在目录（${s.path}），此操作不可恢复。`,
    confirmLabel: "删除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  skillsState.busy[s.path] = true;
  try {
    await invoke("skills_remove", { skillPath: s.path });
    setToast(`已删除技能 ${s.name}`);
    await loadSkills(true);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    skillsState.busy[s.path] = false;
  }
}
</script>

<template>
  <section v-show="active" class="settings-section settings-section-skills">
    <h2 class="settings-section-title">技能管理</h2>
    <p class="settings-section-desc">
      管理 CODEX_HOME 下的本地技能（SKILL.md）
    </p>
    <div class="model-config-card">
      <div class="model-config-card-head">
        <h3>已安装技能</h3>
        <div class="model-config-head-actions">
          <button
            type="button"
            class="model-config-docs-link skill-catalog-link"
            v-tooltip="'Skills Catalog for Codex（浏览器打开）'"
            @click="openDocsUrl(SKILLS_CATALOG_URL)"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_LINK" />
            </svg>
            Skills Catalog for Codex
          </button>
          <button
            class="btn btn-icon primary skill-add-btn"
            v-tooltip="'添加技能'"
            aria-label="添加技能"
            :disabled="skillsState.loading || skillsState.adding"
            @click="addSkill"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_PLUS" />
            </svg>
          </button>
          <button
            class="btn btn-icon model-config-reload-btn"
            aria-label="刷新"
            v-tooltip="'刷新'"
            :disabled="skillsState.loading || skillsState.adding"
            @click="loadSkills()"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_REFRESH" />
            </svg>
          </button>
        </div>
      </div>
      <div class="skills-list">
        <div
          v-if="skillsState.loading && !skillsState.items.length"
          class="plugin-empty"
        >
          正在加载技能…
        </div>
        <div
          v-else-if="!skillsState.items.length && skillsState.errors.length"
          class="plugin-empty"
        >
          {{ skillsState.errors.length }} 个技能因格式问题未加载
          <ul class="skill-error-list">
            <li v-for="e in skillsState.errors" :key="e.path">
              {{ e.message }}（{{ e.path }}）
            </li>
          </ul>
        </div>
        <div v-else-if="!skillsState.items.length" class="plugin-empty">
          暂无可用技能
        </div>
        <div
          v-for="s in skillsState.items"
          :key="s.path"
          class="skill-row"
        >
          <span class="row-icon" aria-hidden="true">
            <svg viewBox="0 0 24 24">
              <path :d="ICON_SKILL" fill="currentColor" />
            </svg>
          </span>
          <div class="skill-info">
            <button
              type="button"
              class="skill-row-main"
              v-tooltip="'在编辑器中打开 SKILL.md'"
              @click="openSkill(s)"
            >
              <span class="skill-name">{{ s.name }}</span>
            </button>
            <p v-if="s.description" class="skill-desc">
              {{ s.description }}
            </p>
          </div>
          <div class="skill-actions">
            <label class="switch">
              <input
                type="checkbox"
                :checked="s.enabled"
                :disabled="skillsState.loading || !!skillsState.busy[s.path]"
                :aria-label="s.enabled ? '禁用技能' : '启用技能'"
                @change="toggleSkill(s)"
              />
              <span class="switch-track"></span>
            </label>
            <button
              type="button"
              class="btn btn-icon danger skill-delete-btn"
              v-tooltip="'删除技能'"
              aria-label="删除技能"
              :disabled="skillsState.loading || !!skillsState.busy[s.path]"
              @click="removeSkill(s)"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_DELETE" />
              </svg>
            </button>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.skill-error-list {
  margin: var(--space-3) 0 0;
  padding: 0;
  list-style: none;
  text-align: left;
  font-size: var(--font-sm);
  font-family: var(--mono);
  color: var(--red);
  word-break: break-all;
  max-height: 120px;
  overflow-y: auto;
}

.skill-error-list li + li {
  margin-top: var(--space-1);
}
</style>

