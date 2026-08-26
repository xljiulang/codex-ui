<script setup lang="ts">
import { computed } from "vue";
import {
  openCommitFileDiffTab,
  type CommitEditorTab,
} from "../composables/useEditorTabs";
import {
  gitStatusLetter,
  normalizeDiffKind,
  type GitCommitFile,
} from "../lib/gitChanges";
import { formatDateTime } from "../lib/format";

const props = defineProps<{ tab: CommitEditorTab }>();

const detail = computed(() => props.tab.detail);

function authorLabel(name: string, email: string): string {
  return email ? `${name} <${email}>` : name;
}

/** 父提交展示：单父显示短哈希；合并提交显示 `短哈希^序号` */
function parentLabel(parents: string[]): string {
  return parents
    .map((p, i) =>
      parents.length > 1 ? `${p.slice(0, 7)}^${i + 1}` : p.slice(0, 7),
    )
    .join(" ");
}

function openFileDiff(file: GitCommitFile) {
  void openCommitFileDiffTab(
    props.tab.workspace,
    props.tab.hash,
    file.path,
    normalizeDiffKind(file.status),
  );
}
</script>

<template>
  <div class="commit-pane">
    <div v-if="tab.loading" class="preview-note">正在加载提交详情…</div>
    <div v-else-if="tab.error" class="preview-note preview-error">
      无法读取提交详情（{{ tab.error }}）
    </div>
    <template v-else-if="detail">
      <div class="commit-head">
        <div class="commit-title-row">
          <span class="commit-subject">{{ detail.subject }}</span>
          <span class="commit-hash" v-tooltip="detail.hash">{{
            detail.shortHash
          }}</span>
        </div>
        <div class="commit-meta">
          <span class="commit-meta-item">
            作者 {{ authorLabel(detail.author, detail.authorEmail) }}
          </span>
          <span class="commit-meta-item">
            提交 {{ authorLabel(detail.committer, detail.committerEmail) }}
          </span>
          <span class="commit-meta-item">
            作者时间 {{ formatDateTime(detail.authorTimeSecs) }}
          </span>
          <span class="commit-meta-item">
            提交时间 {{ formatDateTime(detail.committerTimeSecs) }}
          </span>
          <span v-if="detail.parents.length" class="commit-meta-item">
            父提交 {{ parentLabel(detail.parents) }}
          </span>
        </div>
      </div>

      <div v-if="detail.body" class="commit-body">{{ detail.body }}</div>

      <div class="commit-files">
        <div class="commit-files-title">变更文件（{{ detail.files.length }}）</div>
        <div v-if="detail.files.length" class="commit-file-list">
          <button
            v-for="f in detail.files"
            :key="f.path"
            class="commit-file"
            v-tooltip="f.path"
            @click="openFileDiff(f)"
          >
            <span
              class="commit-file-status"
              :class="'git-status-' + f.status"
            >
              {{ gitStatusLetter(f.status) }}
            </span>
            <span class="commit-file-path">{{ f.path }}</span>
            <span class="commit-file-stats">
              <template v-if="f.binary">二进制</template>
              <template v-else>
                <span class="commit-file-add">+{{ f.insertions }}</span>
                <span class="commit-file-del">-{{ f.deletions }}</span>
              </template>
            </span>
          </button>
        </div>
        <div v-else class="git-section-empty">此提交没有文件变更</div>
      </div>
    </template>
  </div>
</template>
