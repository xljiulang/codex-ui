<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  ICON_CLOSE,
  ICON_OPEN,
  ICON_PLUS,
  ICON_RESTORE,
} from "../lib/icons";
import { setToast, toastError } from "../composables/useCodex";
import {
  applyModelSnapshot,
  createModelSnapshot,
  deleteModelSnapshot,
  listModelSnapshots,
  openModelSnapshot,
} from "../composables/useModelSnapshots";

const open = ref(false);
const loading = ref(false);
const snapshots = ref<string[]>([]);
const newName = ref("");
const root = ref<HTMLElement | null>(null);

/** 打开时拉取快照列表；已打开则关闭。 */
async function toggle() {
  if (open.value) {
    open.value = false;
    return;
  }
  open.value = true;
  try {
    snapshots.value = await listModelSnapshots();
  } catch (e) {
    setToast(toastError(e));
  }
}

/** 新建/覆盖：同名已存在则覆盖更新。 */
async function create(name: string) {
  const n = name.trim();
  if (!n) {
    setToast("请输入快照名");
    return;
  }
  if (loading.value) return;
  loading.value = true;
  try {
    await createModelSnapshot(n);
    newName.value = "";
    snapshots.value = await listModelSnapshots();
  } catch (e) {
    setToast(toastError(e));
  } finally {
    loading.value = false;
  }
}

/** 还原快照：直接写配置文件，重启 codex-ui 后生效。 */
async function apply(name: string) {
  if (loading.value) return;
  loading.value = true;
  try {
    await applyModelSnapshot(name);
    open.value = false;
  } catch (e) {
    setToast(toastError(e));
  } finally {
    loading.value = false;
  }
}

/** 删除快照：组件内确认由 deleteModelSnapshot 的 askConfirm 负责。 */
async function del(name: string) {
  if (loading.value) return;
  loading.value = true;
  try {
    await deleteModelSnapshot(name);
    snapshots.value = await listModelSnapshots();
  } catch (e) {
    setToast(toastError(e));
  } finally {
    loading.value = false;
  }
}

async function openSnapshot(name: string) {
  if (loading.value) return;
  loading.value = true;
  try {
    await openModelSnapshot(name);
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
  <div ref="root" class="model-snapshot-switch" @mousedown.stop>
    <button
      class="icon-btn model-snapshot-btn"
      aria-label="模型快照"
      v-tooltip="'模型快照'"
      :aria-expanded="open ? 'true' : 'false'"
      @click="toggle()"
    >
      <svg class="model-snapshot-icon model-snapshot-brace" viewBox="0 0 16 16" aria-hidden="true">
        <path fill="none" d="M2.2 4.8V2.2H4.8" />
        <path fill="none" d="M13.8 4.8V2.2H11.2" />
        <path fill="none" d="M2.2 11.2V13.8H4.8" />
        <path fill="none" d="M13.8 11.2V13.8H11.2" />
        <path fill="none" d="M5.1 10.6V5.4l2.9 3.2 2.9-3.2v5.2" />
      </svg>
    </button>

    <div v-if="open" class="popup-menu below model-snapshot-menu">
      <div class="model-snapshot-title">
        <svg class="model-snapshot-menu-icon model-snapshot-brace" viewBox="0 0 16 16" aria-hidden="true">
          <path fill="none" d="M2.2 4.8V2.2H4.8" />
          <path fill="none" d="M13.8 4.8V2.2H11.2" />
          <path fill="none" d="M2.2 11.2V13.8H4.8" />
          <path fill="none" d="M13.8 11.2V13.8H11.2" />
          <path fill="none" d="M5.1 10.6V5.4l2.9 3.2 2.9-3.2v5.2" />
        </svg>
        <span>模型快照</span>
      </div>

      <div v-if="snapshots.length === 0" class="git-branch-menu-empty">暂无模型快照</div>

      <div
        v-for="p in snapshots"
        :key="p"
        class="model-snapshot-row popup-menu-item"
      >
        <span class="git-branch-name">{{ p }}</span>
        <button
          class="model-snapshot-apply-btn"
          :aria-label="`还原模型快照${p}`"
          v-tooltip="'还原'"
          :disabled="loading"
          @click="apply(p)"
        >
          <svg viewBox="0 0 24 24">
            <path :d="ICON_RESTORE" />
          </svg>
        </button>
        <button
          class="git-branch-delete"
          :aria-label="`删除模型快照${p}`"
          v-tooltip="'删除'"
          :disabled="loading"
          @click="del(p)"
        >
          <svg viewBox="0 0 24 24">
            <path :d="ICON_CLOSE" />
          </svg>
        </button>
        <button
          class="model-snapshot-open-btn"
          :aria-label="`打开模型快照${p}`"
          v-tooltip="'打开'"
          :disabled="loading"
          @click="openSnapshot(p)"
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
          placeholder="新建模型快照"
          :disabled="loading"
          @keyup.enter="onNew()"
        />
        <button
          class="btn sm git-branch-create-btn"
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
.model-snapshot-switch {
  position: relative;
  display: flex;
  align-items: center;
}

.model-snapshot-btn {
  position: relative;
}

.model-snapshot-icon,
.model-snapshot-menu-icon {
  display: block;
}

.model-snapshot-icon {
  width: 16px;
  height: 16px;
}

.model-snapshot-menu-icon {
  width: 15px;
  height: 15px;
}

/* 单色外框 + 字段 M：stroke 用 currentColor 跟随按钮/标题颜色 */
.model-snapshot-brace path {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.4;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.model-snapshot-menu {
  right: 0;
  left: auto;
  min-width: 300px;
  padding: var(--space-2);
}

.model-snapshot-title {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-1) var(--space-3) 2px;
  font-size: var(--font-sm);
  font-weight: 700;
  color: var(--text-faint);
  letter-spacing: 0.5px;
}

.model-snapshot-row {
  cursor: default;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
}

.model-snapshot-apply-btn,
.model-snapshot-open-btn {
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

.model-snapshot-apply-btn:hover:not(:disabled),
.model-snapshot-open-btn:hover:not(:disabled) {
  color: var(--accent);
  background: rgba(var(--accent-rgb), 0.14);
}

.model-snapshot-apply-btn:disabled,
.model-snapshot-open-btn:disabled {
  opacity: var(--opacity-disabled);
  cursor: default;
}

.model-snapshot-apply-btn svg,
.model-snapshot-open-btn svg {
  width: 12px;
  height: 12px;
  fill: currentColor;
}
</style>
