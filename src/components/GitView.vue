<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { store, toastError } from "../composables/useCodex";
import {
  gitErrorMsg,
  gitInitBusy,
  gitState,
  gitStatus,
  initGitRepo,
  refreshGitChanges,
  setGitChangesActive,
} from "../composables/useGitChanges";
import { sessionRoot } from "../composables/useSessionFs";
import { gitDiffKind, gitStatusLabel, type GitFile } from "../lib/gitChanges";

const props = defineProps<{ active: boolean }>();

watch(
  () => props.active,
  (v) => setGitChangesActive(v),
  { immediate: true },
);

const confirmInit = ref(false);

const branchLabel = computed(() => gitStatus.value?.branch ?? "");
const repoRoot = computed(() => gitStatus.value?.repoRoot ?? "");
const fileCount = computed(() => gitStatus.value?.files.length ?? 0);

function startInit() {
  confirmInit.value = true;
}

async function doInit() {
  confirmInit.value = false;
  await initGitRepo();
}

async function openDiff(file: GitFile) {
  const root = gitStatus.value?.repoRoot;
  if (!root) return;
  try {
    const diff = await invoke<string>("git_changes_diff", {
      root,
      path: file.path,
      kind: file.status,
    });
    if (!diff) {
      store.toast = "该文件无内容变化（可能仅为重命名）";
      return;
    }
    await invoke("open_diff_window", {
      params: {
        path: file.path,
        kind: gitDiffKind(file.status),
        diff,
        workspace_root: root,
      },
    });
  } catch (e) {
    store.toast = toastError(e);
  }
}
</script>

<template>
  <div class="git-view">
    <div v-if="gitState === 'loading'" class="git-note">正在检查 Git 状态…</div>

    <div v-else-if="gitState === 'no_git'" class="git-empty">
      <p class="git-empty-title">未检测到 Git</p>
      <p class="git-empty-desc">
        当前环境未安装 Git，无法查看文件更改。请安装 Git 后重试。
      </p>
    </div>

    <div v-else-if="gitState === 'not_repo'" class="git-empty">
      <p class="git-empty-title">当前目录不是 Git 仓库</p>
      <p class="git-empty-desc">
        点击下方按钮可将当前工作目录初始化为 Git 仓库（仅初始化，不会自动提交）。
      </p>
      <button
        class="btn git-init-btn"
        :disabled="gitInitBusy"
        @click="startInit"
      >
        {{ gitInitBusy ? "初始化中…" : "添加到 Git" }}
      </button>
    </div>

    <div v-else-if="gitState === 'error'" class="git-empty">
      <p class="git-empty-title">无法获取 Git 状态</p>
      <p class="git-empty-desc">{{ gitErrorMsg }}</p>
      <button class="btn git-init-btn" @click="refreshGitChanges()">重试</button>
    </div>

    <template v-else-if="gitState === 'ok' && gitStatus">
      <div class="git-head">
        <span class="git-branch" :title="repoRoot">{{ branchLabel }}</span>
        <span class="git-count">{{ fileCount }} 个更改</span>
        <button
          class="git-refresh"
          aria-label="刷新"
          title="刷新"
          @click="refreshGitChanges()"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M17.65 6.35A7.95 7.95 0 0 0 12 4a8 8 0 1 0 7.73 10h-2.08A6 6 0 1 1 12 6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z"
            />
          </svg>
        </button>
      </div>

      <ul v-if="fileCount" class="git-file-list">
        <li
          v-for="file in gitStatus.files"
          :key="file.path"
          class="git-file"
          :title="`查看 ${file.path} 的更改`"
          @click="openDiff(file)"
        >
          <span class="git-badge" :class="`git-badge-${file.status}`">
            {{ gitStatusLabel(file.status) }}
          </span>
          <span class="git-path">{{ file.path }}</span>
        </li>
      </ul>
      <div v-else class="git-note">当前没有更改</div>
    </template>

    <div v-if="confirmInit" class="modal-mask" @click.self="confirmInit = false">
      <div class="modal" tabindex="-1">
        <div class="modal-head">
          <span class="modal-title">添加到 Git</span>
          <button
            class="modal-close"
            aria-label="关闭"
            @click="confirmInit = false"
          >
            ×
          </button>
        </div>
        <div class="modal-body">
          确定要将当前目录初始化为 Git 仓库吗？<br />
          <span class="git-confirm-path">
            {{ sessionRoot || "当前工作目录" }}
          </span>
          <p class="git-confirm-desc">
            将执行 git init，仅初始化、不会自动提交；初始化后现有文件会以“未跟踪”状态显示。
          </p>
        </div>
        <div class="modal-foot">
          <button class="btn" @click="confirmInit = false">取消</button>
          <button class="btn git-init-ok" :disabled="gitInitBusy" @click="doInit">
            {{ gitInitBusy ? "初始化中…" : "确认" }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
