<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  ICON_CLOSE,
  ICON_OPEN,
  ICON_PLUS,
  ICON_RESTORE,
} from "../lib/icons";
import { setToast, store, toastError } from "../composables/useCodex";
import {
  applyConfigProfile,
  createConfigProfile,
  deleteConfigProfile,
  listConfigProfiles,
  openConfigProfile,
} from "../composables/useConfigProfiles";

const open = ref(false);
const loading = ref(false);
const profiles = ref<string[]>([]);
const newName = ref("");
const root = ref<HTMLElement | null>(null);

const activeConfig = computed(() => store.settings.active_config ?? null);

/** 打开时拉取配置列表；已打开则关闭。 */
async function toggle() {
  if (open.value) {
    open.value = false;
    return;
  }
  open.value = true;
  try {
    profiles.value = await listConfigProfiles();
  } catch (e) {
    setToast(toastError(e));
  }
}

/** 新建/覆盖：同名已存在则覆盖更新。 */
async function create(name: string) {
  const n = name.trim();
  if (!n) {
    setToast("请输入配置名");
    return;
  }
  if (loading.value) return;
  loading.value = true;
  try {
    await createConfigProfile(n);
    newName.value = "";
    profiles.value = await listConfigProfiles();
    setToast(`已创建并切换到配置快照「${n}」`);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    loading.value = false;
  }
}

/** 应用配置：覆盖 codex-home 文件 + 热重载 + 记录激活。 */
async function apply(name: string) {
  if (loading.value) return;
  loading.value = true;
  try {
    await applyConfigProfile(name);
    open.value = false;
  } catch (e) {
    setToast(toastError(e));
  } finally {
    loading.value = false;
  }
}

/** 删除配置：组件内确认由 deleteConfigProfile 的 askConfirm 负责。 */
async function del(name: string) {
  if (loading.value) return;
  loading.value = true;
  try {
    await deleteConfigProfile(name);
    profiles.value = await listConfigProfiles();
  } catch (e) {
    setToast(toastError(e));
  } finally {
    loading.value = false;
  }
}

function onNew() {
  void create(newName.value);
}

function onClickOutside(e: MouseEvent) {
  if (!open.value) return;
  const el = root.value;
  if (el && !el.contains(e.target as Node)) open.value = false;
}

onMounted(() => document.addEventListener("mousedown", onClickOutside));
onBeforeUnmount(() => document.removeEventListener("mousedown", onClickOutside));

watch(open, (v) => {
  if (!v) newName.value = "";
});
</script>

<template>
  <div ref="root" class="config-switch" @mousedown.stop>
    <button
      class="icon-btn config-switch-btn"
      aria-label="配置快照"
      v-tooltip="'配置快照'"
      :aria-expanded="open ? 'true' : 'false'"
      @click="toggle()"
    >
      <svg class="config-switch-icon config-brace" viewBox="0 0 16 16" aria-hidden="true">
        <path fill="none" d="M6.5 3.1c-1.4 0-2.1.8-2.1 2.2v1c0 .6-.4 1.1-1 1.2v.9c.6.1 1 .6 1 1.2v1c0 1.4.7 2.2 2.1 2.2" />
        <path fill="none" d="M9.5 3.1c1.4 0 2.1.8 2.1 2.2v1c0 .6.4 1.1 1 1.2v.9c-.6.1-1 .6-1 1.2v1c0 1.4-.7 2.2-2.1 2.2" />
      </svg>
    </button>

    <div v-if="open" class="popup-menu below config-profiles-menu">
      <div class="config-menu-title">
        <svg class="config-menu-icon config-brace" viewBox="0 0 16 16" aria-hidden="true">
          <path fill="none" d="M6.5 3.1c-1.4 0-2.1.8-2.1 2.2v1c0 .6-.4 1.1-1 1.2v.9c.6.1 1 .6 1 1.2v1c0 1.4.7 2.2 2.1 2.2" />
          <path fill="none" d="M9.5 3.1c1.4 0 2.1.8 2.1 2.2v1c0 .6.4 1.1 1 1.2v.9c-.6.1-1 .6-1 1.2v1c0 1.4-.7 2.2-2.1 2.2" />
        </svg>
        <span>配置快照</span>
      </div>

      <div v-if="profiles.length === 0" class="git-branch-menu-empty">暂无配置快照</div>

      <div
        v-for="p in profiles"
        :key="p"
        class="git-branch-menu-item config-profile-row"
        :class="{ current: p === activeConfig }"
      >
        <span class="git-branch-name">{{ p }}</span>
        <button
          class="config-apply-btn"
          :class="{ active: p === activeConfig }"
          :aria-label="`还原配置快照${p}`"
          v-tooltip="'还原配置快照'"
          :disabled="loading"
          @click="apply(p)"
        >
          <svg viewBox="0 0 24 24">
            <path :d="ICON_RESTORE" />
          </svg>
        </button>
        <button
          class="git-branch-delete"
          :aria-label="`删除配置快照${p}`"
          v-tooltip="'删除配置快照'"
          :disabled="loading"
          @click="del(p)"
        >
          <svg viewBox="0 0 24 24">
            <path :d="ICON_CLOSE" />
          </svg>
        </button>
        <button
          class="config-open-btn"
          :aria-label="`打开配置快照${p}`"
          v-tooltip="'打开配置快照'"
          :disabled="loading"
          @click="openConfigProfile(p)"
        >
          <svg viewBox="0 0 24 24">
            <path :d="ICON_OPEN" />
          </svg>
        </button>
      </div>

      <div class="git-branch-create">
        <input
          v-model="newName"
          class="git-branch-input"
          placeholder="新建配置快照"
          :disabled="loading"
          @keyup.enter="onNew()"
        />
        <button
          class="git-branch-create-btn"
          :disabled="loading || !newName.trim()"
          @click="onNew()"
        >
          <svg viewBox="0 0 24 24">
            <path :d="ICON_PLUS" />
          </svg>
          新建
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.config-switch {
  position: relative;
  display: flex;
  align-items: center;
}

.config-switch-btn {
  position: relative;
}

.config-switch-icon,
.config-menu-icon {
  display: block;
}

.config-switch-icon {
  width: 16px;
  height: 16px;
}

.config-menu-icon {
  width: 15px;
  height: 15px;
}

/* 单色花括号：stroke 用 currentColor 跟随按钮/标题颜色（与相邻 .icon-btn 一致） */
.config-brace path {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.4;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.config-profiles-menu {
  right: 0;
  left: auto;
  min-width: 280px;
  padding: var(--space-2);
}

.config-menu-title {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-1) var(--space-2);
  font-weight: 600;
  color: var(--text-bright);
}

.config-profile-row {
  cursor: default;
}

.config-apply-btn {
  flex-shrink: 0;
  width: 20px;
  height: 20px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: var(--radius);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
}

.config-apply-btn:hover:not(:disabled) {
  color: var(--accent);
  background: rgba(var(--accent-rgb), 0.14);
}

.config-apply-btn.active {
  color: var(--accent);
}

.config-apply-btn:disabled {
  opacity: var(--opacity-disabled);
  cursor: default;
}

.config-apply-btn svg {
  width: 12px;
  height: 12px;
  fill: currentColor;
}

.config-open-btn {
  flex-shrink: 0;
  width: 20px;
  height: 20px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: var(--radius);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
}

.config-open-btn:hover:not(:disabled) {
  color: var(--accent);
  background: rgba(var(--accent-rgb), 0.14);
}

.config-open-btn:disabled {
  opacity: var(--opacity-disabled);
  cursor: default;
}

.config-open-btn svg {
  width: 12px;
  height: 12px;
  fill: currentColor;
}
</style>
