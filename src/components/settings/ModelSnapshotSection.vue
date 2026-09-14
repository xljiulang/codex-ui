<script setup lang="ts">
// 模型快照分区（设置 → 模型快照）：列出/新建/还原/删除/打开 CODEX_HOME/codex-ui/*.json。
// 还原成功后 emit("applied")，由设置页转告「模型配置」分区重读一次（磁盘值即时同步）。
import { nextTick, ref, watch } from "vue";
import ModalDialog from "../ModalDialog.vue";
import { setToast, toastError } from "../../composables/useCodex";
import {
  ICON_DELETE,
  ICON_OPEN,
  ICON_PLUS,
  ICON_REFRESH,
  ICON_RESTORE,
} from "../../lib/icons";
import {
  applyModelSnapshot,
  createModelSnapshot,
  deleteModelSnapshot,
  listModelSnapshots,
  openModelSnapshot,
} from "../../composables/useModelSnapshots";

const props = defineProps<{ active: boolean }>();
const emit = defineEmits<{ applied: [] }>();

const loading = ref(false);
const snapshots = ref<string[]>([]);
const creating = ref(false);
const draft = ref("");
const nameInput = ref<HTMLInputElement | null>(null);

/** 拉取快照列表；非数组结果（测试 mock 返回 undefined）兜底为空列表 */
async function load() {
  loading.value = true;
  try {
    snapshots.value = (await listModelSnapshots()) ?? [];
  } catch (e) {
    setToast(toastError(e));
  } finally {
    loading.value = false;
  }
}

/** 打开新建弹窗：清空草稿并在渲染后聚焦输入框 */
function openCreate() {
  if (loading.value) return;
  draft.value = "";
  creating.value = true;
  void nextTick(() => nameInput.value?.focus());
}

function cancelCreate() {
  creating.value = false;
}

/** 新建/覆盖：同名已存在则覆盖更新，成功后关闭弹窗并刷新列表 */
async function confirmCreate() {
  const name = draft.value.trim();
  if (!name) {
    setToast("请输入快照名");
    return;
  }
  if (loading.value) return;
  loading.value = true;
  try {
    await createModelSnapshot(name);
    creating.value = false;
    await load();
  } catch (e) {
    setToast(toastError(e));
  } finally {
    loading.value = false;
  }
}

/** 还原快照：写回磁盘配置并通知设置页转告「模型配置」分区重读 */
async function apply(name: string) {
  if (loading.value) return;
  loading.value = true;
  try {
    await applyModelSnapshot(name);
    emit("applied");
  } catch (e) {
    setToast(toastError(e));
  } finally {
    loading.value = false;
  }
}

/** 删除快照：确认弹窗由 deleteModelSnapshot 内部负责 */
async function del(name: string) {
  if (loading.value) return;
  loading.value = true;
  try {
    await deleteModelSnapshot(name);
    await load();
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

// 切换到「模型快照」Tab 时加载一次列表
watch(
  () => props.active,
  (active) => {
    if (active) void load();
  },
  { immediate: true },
);
</script>

<template>
  <section v-show="active" class="settings-section settings-section-model-snapshot">
    <h2 class="settings-section-title">模型快照</h2>
    <p class="settings-section-desc">
      保存、还原与删除模型配置快照（CODEX_HOME/codex-ui）
    </p>

    <div class="model-config-card">
      <div class="model-config-card-head">
        <h3>已保存快照</h3>
        <div class="model-config-head-actions">
          <button
            class="btn btn-icon primary model-snapshot-add-btn"
            type="button"
            aria-label="新建模型快照"
            v-tooltip="'新建模型快照'"
            :disabled="loading"
            @click="openCreate"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_PLUS" />
            </svg>
          </button>
          <button
            class="btn btn-icon model-config-reload-btn"
            type="button"
            aria-label="刷新"
            v-tooltip="'刷新'"
            :disabled="loading"
            @click="load"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_REFRESH" />
            </svg>
          </button>
        </div>
      </div>

      <div class="model-snapshot-list">
        <div v-if="loading && !snapshots.length" class="plugin-empty">
          正在加载模型快照…
        </div>
        <div v-else-if="!snapshots.length" class="plugin-empty">
          暂无模型快照
        </div>
        <div
          v-for="name in snapshots"
          :key="name"
          class="model-snapshot-row"
        >
          <span class="model-snapshot-name">{{ name }}</span>
          <div class="model-snapshot-actions">
            <button
              class="btn btn-icon"
              type="button"
              :aria-label="`还原模型快照${name}`"
              v-tooltip="'还原'"
              :disabled="loading"
              @click="apply(name)"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_RESTORE" />
              </svg>
            </button>
            <button
              class="btn btn-icon"
              type="button"
              :aria-label="`打开模型快照${name}`"
              v-tooltip="'打开'"
              :disabled="loading"
              @click="openSnapshot(name)"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_OPEN" />
              </svg>
            </button>
            <button
              class="btn btn-icon danger"
              type="button"
              :aria-label="`删除模型快照${name}`"
              v-tooltip="'删除'"
              :disabled="loading"
              @click="del(name)"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_DELETE" />
              </svg>
            </button>
          </div>
        </div>
      </div>

      <ModalDialog
        v-if="creating"
        title="新建模型快照"
        closable
        @close="cancelCreate"
      >
        <div class="setting-row">
          <label for="model-snapshot-name">
            快照名称
            <span class="model-config-required" aria-label="必填" v-tooltip="'必填'">*</span>
          </label>
          <input
            id="model-snapshot-name"
            ref="nameInput"
            v-model="draft"
            type="text"
            placeholder="如 dev"
            :disabled="loading"
            @keyup.enter="confirmCreate"
          />
          <p class="model-snapshot-hint">
            同名快照会被覆盖；快照包含 API Key，请谨慎分享。
          </p>
        </div>
        <template #foot>
          <button class="btn" type="button" @click="cancelCreate">取消</button>
          <button
            class="btn primary model-snapshot-create-btn"
            type="button"
            :disabled="loading || !draft.trim()"
            @click="confirmCreate"
          >
            确定
          </button>
        </template>
      </ModalDialog>
    </div>
  </section>
</template>

<style scoped>
/* 弹窗内提示文案：与设置页其它说明文字同色同级（BasicSection .setting-note 同款取值） */
.model-snapshot-hint {
  margin: var(--space-2) 0 0;
  font-size: var(--font-sm);
  line-height: 1.5;
  color: var(--text-dim);
  word-break: break-all;
  user-select: text;
}
</style>
