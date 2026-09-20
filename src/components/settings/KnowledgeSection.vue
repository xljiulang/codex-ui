<script setup lang="ts">
import { onMounted } from "vue";
import { askConfirm, setToast, toastError } from "../../composables/useCodex";
import {
  deleteKnowledge,
  knowledgeKbs,
  knowledgeProgress,
  refreshKnowledgeList,
  type KnowledgeKb,
} from "../../composables/useKnowledge";
import { ICON_DELETE, ICON_TOOL } from "../../lib/icons";

defineProps<{ active: boolean }>();

/** 上次更新时间文案（0 = 从未索引） */
function updatedText(ts: number): string {
  if (!ts) return "未索引";
  return new Date(ts * 1000).toLocaleString();
}

/** 该库当前是否有进行中的建库任务 */
function progressText(kb: KnowledgeKb): string {
  const key = kb.cwd.trim().replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
  return knowledgeProgress.value[key] ?? "";
}

onMounted(() => {
  void refreshKnowledgeList();
});

async function onDelete(kb: KnowledgeKb) {
  const ok = await askConfirm({
    title: "删除知识库",
    message:
      `确定删除工作目录「${kb.cwd || kb.file}」的知识库索引吗？\n` +
      "只删除本地索引（含向量与关键词索引），不影响原始文档与向量模型；下次可用 index_docs 重新建库。",
    confirmLabel: "删除",
  });
  if (!ok) return;
  try {
    await deleteKnowledge(kb.file);
  } catch (e) {
    setToast(toastError(e));
  }
}
</script>

<template>
  <section v-show="active" class="settings-section settings-section-knowledge">
    <h2 class="settings-section-title">知识库</h2>
    <p class="settings-section-desc">
      按会话工作目录一对一存放（同一目录的多个会话共享一个知识库）。检索与建库由动态工具
      <code>codexui.search_docs</code> / <code>codexui.index_docs</code> 完成——两者默认关闭，
      需在「动态工具」中开启后新建会话才可用
    </p>
    <div class="model-config-card">
      <div class="model-config-card-head">
        <h3>已建立的知识库</h3>
        <div class="model-config-head-actions">
          <span class="knowledge-hint">索引数据位于应用数据目录 knowledge/</span>
        </div>
      </div>
      <div v-if="!knowledgeKbs.length" class="knowledge-empty">
        暂无知识库：在会话里让 codex 调用 index_docs 建立
      </div>
      <div v-else class="skills-list">
        <div v-for="kb in knowledgeKbs" :key="kb.file" class="knowledge-row">
          <span class="row-icon" aria-hidden="true">
            <svg viewBox="0 0 24 24">
              <path :d="ICON_TOOL" fill="currentColor" />
            </svg>
          </span>
          <div class="skill-info">
            <span class="skill-row-main">
              <span class="skill-name">{{ kb.cwd || kb.file }}</span>
            </span>
            <p class="skill-desc">
              <template v-if="kb.available">
                {{ kb.docs }} 篇文档 · {{ kb.chunks }} 个切块 · 上次更新
                {{ updatedText(kb.updated_at) }}
                <template v-if="progressText(kb)">
                  · {{ progressText(kb) }}
                </template>
              </template>
              <template v-else>
                不可用：{{ kb.error || "元数据缺失" }}
              </template>
            </p>
          </div>
          <div class="skill-actions">
            <button
              type="button"
              class="btn btn-icon danger knowledge-delete-btn"
              v-tooltip="'删除知识库'"
              :aria-label="`删除知识库 ${kb.cwd || kb.file}`"
              @click="onDelete(kb)"
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
/* 分区头部提示：与动态工具分区同款小字 */
.knowledge-hint {
  font-size: var(--font-xs);
  color: var(--text-faint);
  white-space: nowrap;
}

.knowledge-empty {
  padding: var(--space-4);
  color: var(--text-faint);
  font-size: var(--font-sm);
}

.knowledge-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-4);
}

.knowledge-row + .knowledge-row {
  border-top: 1px solid var(--border);
}
</style>
