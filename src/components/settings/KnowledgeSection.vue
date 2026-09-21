<script setup lang="ts">
import { onMounted } from "vue";
import { askConfirm, setToast, toastError } from "../../composables/useCodex";
import {
  deleteKnowledge,
  knowledgeKbs,
  knowledgeProgress,
  normalizeKbKey,
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
  if (!kb.kb) return "";
  return knowledgeProgress.value[normalizeKbKey(kb.kb)] ?? "";
}

onMounted(() => {
  void refreshKnowledgeList();
});

async function onDelete(kb: KnowledgeKb) {
  const label = kb.kb || kb.file;
  const ok = await askConfirm({
    title: "删除知识库",
    message:
      `确定删除知识库「${label}」的索引吗？\n` +
      `索引来源目录：${kb.source || "（未知）"}\n` +
      "只删除本地索引（含向量与关键词索引），不影响原始文档与向量模型；\
       之后可用 codexui-kb create + index 重新建库。",
    confirmLabel: "删除",
  });
  if (!ok) return;
  try {
    await deleteKnowledge(kb);
  } catch (e) {
    setToast(toastError(e));
  }
}
</script>

<template>
  <section v-show="active" class="settings-section settings-section-knowledge">
    <h2 class="settings-section-title">知识库</h2>
    <p class="settings-section-desc">
      知识库按<b>库名</b>管理，每个库登记一个索引来源目录；跨目录共享同一个库时使用同一个库名。
      索引与检索由 <code>codexui-kb</code> 命令行完成（由 skill 调用）：
      <code>create &lt;库名&gt; &lt;目录&gt;</code> 登记来源、
      <code>index &lt;库名&gt;</code> 建库、<code>search &lt;库名&gt; --query &lt;词&gt;</code> 检索。
    </p>
    <div class="model-config-card">
      <div class="model-config-card-head">
        <h3>已建立的知识库</h3>
        <div class="model-config-head-actions">
          <span class="knowledge-hint">索引数据位于应用数据目录 kbs/</span>
        </div>
      </div>
      <div v-if="!knowledgeKbs.length" class="knowledge-empty">
        暂无知识库：用 codexui-kb create &lt;库名&gt; &lt;目录&gt; 建立
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
              <span class="skill-name">{{ kb.kb || kb.file }}</span>
            </span>
            <p class="skill-desc">
              <template v-if="kb.available">
                来源：{{ kb.source || "（未知）" }} · {{ kb.docs }} 篇文档 · {{ kb.chunks }} 个切块 · 上次更新
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
              :aria-label="`删除知识库 ${kb.kb || kb.file}`"
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
