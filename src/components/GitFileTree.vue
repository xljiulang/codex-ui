<script setup lang="ts">
import type { GitFile } from "../lib/gitChanges";
import { gitStatusLetter } from "../lib/gitChanges";
import type { GitTreeNode } from "../lib/gitTree";
import {
  ICON_ARROW_DOWN,
  ICON_ARROW_RIGHT,
  ICON_FILE,
  ICON_FOLDER_CLOSED,
  ICON_FOLDER_OPEN,
} from "../lib/icons";

defineProps<{
  /** 扁平可见行（buildGitTree + flattenRows 的结果） */
  rows: GitTreeNode[];
  /** 所属分区（key 前缀 + 右键菜单分发用） */
  section: "changes" | "staged";
  /** 当前高亮的变更文件（相对仓库根路径） */
  selectedPath: string;
  /** 文件类型图标（未缓存/取不到返回 undefined，渲染层回退 SVG） */
  fileIcon: (file: GitFile) => string | undefined;
  /** 空列表占位文案 */
  emptyText: string;
}>();

defineEmits<{
  rowClick: [row: GitTreeNode];
  rowContext: [section: "changes" | "staged", row: GitTreeNode, e: MouseEvent];
}>();
</script>

<template>
  <div v-if="rows.length" class="git-file-list">
    <div
      v-for="row in rows"
      :key="`${section}:${row.kind}:${row.relPath}`"
      class="git-tree-row"
      :class="{
        'git-dir': row.kind === 'dir',
        'git-file': row.kind === 'file',
        selected: row.kind === 'file' && row.relPath === selectedPath,
      }"
      :data-git-path="row.kind === 'file' ? row.relPath : undefined"
      :style="{ paddingLeft: 8 + row.depth * 14 + 'px' }"
      @click="$emit('rowClick', row)"
      @contextmenu="$emit('rowContext', section, row, $event)"
    >
      <template v-if="row.kind === 'dir'">
        <svg class="git-dir-arrow" viewBox="0 0 24 24" aria-hidden="true">
          <path :d="row.collapsed ? ICON_ARROW_RIGHT : ICON_ARROW_DOWN" />
        </svg>
        <svg class="git-dir-icon" viewBox="0 0 24 24" aria-hidden="true">
          <path :d="row.collapsed ? ICON_FOLDER_CLOSED : ICON_FOLDER_OPEN" />
        </svg>
        <span class="git-dir-name">{{ row.name }}</span>
      </template>
      <template v-else>
        <span class="git-file-icon" aria-hidden="true">
          <img
            v-if="fileIcon(row.file)"
            class="git-file-icon-img"
            :src="fileIcon(row.file)"
            alt=""
            draggable="false"
          />
          <svg v-else viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_FILE" />
          </svg>
        </span>
        <span
          class="git-path"
          :class="{ 'git-path-strike': row.file.status === 'deleted' }"
        >
          {{ row.file.path }}
        </span>
        <span
          class="git-status-icon"
          :class="`git-status-${row.file.status}`"
        >
          {{ gitStatusLetter(row.file.status) }}
        </span>
      </template>
    </div>
  </div>
  <div v-else class="git-section-empty">{{ emptyText }}</div>
</template>
