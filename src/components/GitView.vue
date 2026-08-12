<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
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
import {
  gitDiffKind,
  gitStatusIcon,
  gitStatusLabel,
  type GitFile,
  type GitStatus,
} from "../lib/gitChanges";

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
const branchMenuOpen = ref(false);
const branches = ref<string[]>([]);
const newBranchName = ref("");
const branchBusy = ref(false);

function startInit() {
  confirmInit.value = true;
}

async function doInit() {
  confirmInit.value = false;
  await initGitRepo();
}

async function openBranchMenu() {
  const root = repoRoot.value;
  if (!root || branchBusy.value) return;
  branchBusy.value = true;
  try {
    const res = await invoke<{ current: string; branches: string[] }>(
      "git_changes_branches",
      { path: root },
    );
    branches.value = res.branches;
    branchMenuOpen.value = true;
  } catch (e) {
    store.toast = toastError(e);
  } finally {
    branchBusy.value = false;
  }
}

async function toggleBranchMenu() {
  if (branchMenuOpen.value) {
    branchMenuOpen.value = false;
  } else {
    await openBranchMenu();
  }
}

async function switchBranch(name: string) {
  if (branchBusy.value || name === branchLabel.value) return;
  branchBusy.value = true;
  try {
    const st = await invoke<GitStatus>("git_changes_branch_switch", {
      path: repoRoot.value,
      name,
    });
    gitStatus.value = st;
    branchMenuOpen.value = false;
  } catch (e) {
    store.toast = toastError(e);
  } finally {
    branchBusy.value = false;
  }
}

async function createBranch() {
  const name = newBranchName.value.trim();
  if (!name || branchBusy.value) return;
  branchBusy.value = true;
  try {
    const st = await invoke<GitStatus>("git_changes_branch_create", {
      path: repoRoot.value,
      name,
    });
    gitStatus.value = st;
    newBranchName.value = "";
    // 重新拉取分支列表，保持弹层打开
    const res = await invoke<{ current: string; branches: string[] }>(
      "git_changes_branches",
      { path: repoRoot.value },
    );
    branches.value = res.branches;
  } catch (e) {
    store.toast = toastError(e);
  } finally {
    branchBusy.value = false;
  }
}

async function deleteBranch(name: string) {
  if (branchBusy.value || name === branchLabel.value) return;
  branchBusy.value = true;
  try {
    const st = await invoke<GitStatus>("git_changes_branch_delete", {
      path: repoRoot.value,
      name,
    });
    gitStatus.value = st;
    branches.value = branches.value.filter((b) => b !== name);
  } catch (e) {
    store.toast = toastError(e);
  } finally {
    branchBusy.value = false;
  }
}

function onWindowClick(e: MouseEvent) {
  // 用 Element 而非 HTMLElement：点击 svg/path 等 SVG 目标也应正确判断
  if (!(e.target instanceof Element)) {
    branchMenuOpen.value = false;
    return;
  }
  if (e.target.closest(".git-branch-menu") || e.target.closest(".git-branch-btn")) {
    return;
  }
  branchMenuOpen.value = false;
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") branchMenuOpen.value = false;
}

function onWindowScroll(e: Event) {
  // 仅面板自身滚动时关闭；聊天区等外部滚动不影响分支弹层
  if (!(e.target instanceof Element)) return;
  if (!e.target.closest(".git-view")) return;
  branchMenuOpen.value = false;
}

onMounted(() => {
  window.addEventListener("click", onWindowClick);
  window.addEventListener("keydown", onKeydown);
  window.addEventListener("scroll", onWindowScroll, true);
});
onBeforeUnmount(() => {
  window.removeEventListener("click", onWindowClick);
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("scroll", onWindowScroll, true);
});

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
        <button
          class="git-branch git-branch-btn"
          v-tooltip="repoRoot"
          :disabled="branchBusy"
          @click="toggleBranchMenu()"
        >
          <svg class="git-branch-icon" viewBox="0 0 16 16" aria-hidden="true">
            <path
              d="M14 5.5C14 4.121 12.879 3 11.5 3C10.121 3 9 4.121 9 5.5C9 6.682 9.826 7.669 10.93 7.928C10.744 8.546 10.177 9 9.5 9H6.5C5.935 9 5.419 9.195 5 9.512V4.949C6.14 4.717 7 3.707 7 2.5C7 1.121 5.879 0 4.5 0C3.121 0 2 1.121 2 2.5C2 3.708 2.86 4.717 4 4.949V11.05C2.86 11.282 2 12.292 2 13.499C2 14.878 3.121 15.999 4.5 15.999C5.879 15.999 7 14.878 7 13.499C7 12.317 6.174 11.33 5.07 11.071C5.256 10.453 5.823 9.999 6.5 9.999H9.5C10.723 9.999 11.74 9.115 11.954 7.953C13.116 7.738 14 6.723 14 5.5ZM3 2.5C3 1.673 3.673 1 4.5 1C5.327 1 6 1.673 6 2.5C6 3.327 5.327 4 4.5 4C3.673 4 3 3.327 3 2.5ZM6 13.5C6 14.327 5.327 15 4.5 15C3.673 15 3 14.327 3 13.5C3 12.673 3.673 12 4.5 12C5.327 12 6 12.673 6 13.5ZM11.5 7C10.673 7 10 6.327 10 5.5C10 4.673 10.673 4 11.5 4C12.327 4 13 4.673 13 5.5C13 6.327 12.327 7 11.5 7Z"
            />
          </svg>
          <span class="git-branch-name-text">{{ branchLabel }}</span>
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M7.41 8.59L12 13.17l4.59-4.58L18 10l-6 6-6-6 1.41-1.41z"
            />
          </svg>
        </button>
        <span class="git-count">{{ fileCount }} 个更改</span>
        <button
          class="git-refresh"
          aria-label="刷新"
          v-tooltip="'刷新'"
          @click="refreshGitChanges()"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M17.65 6.35A7.95 7.95 0 0 0 12 4a8 8 0 1 0 7.73 10h-2.08A6 6 0 1 1 12 6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z"
            />
          </svg>
        </button>
        <!-- 分支弹层作为 .git-head 子节点，absolute 定位相对头部，避免被面板 overflow 裁掉 -->
        <div v-if="branchMenuOpen" class="git-branch-menu">
          <div class="git-branch-menu-list">
            <div
              v-for="b in branches"
              :key="b"
              class="git-branch-menu-item"
              :class="{ current: b === branchLabel }"
              v-tooltip="b === branchLabel ? '当前分支' : `切换到 ${b}`"
              @click="switchBranch(b)"
            >
              <span class="git-branch-check">
                {{ b === branchLabel ? "✓" : "" }}
              </span>
              <span class="git-branch-name">{{ b }}</span>
              <button
                v-if="b !== branchLabel"
                class="git-branch-delete"
                v-tooltip="'删除分支'"
                :disabled="branchBusy"
                @click.stop="deleteBranch(b)"
              >
                ×
              </button>
            </div>
            <div v-if="!branches.length" class="git-branch-menu-empty">
              暂无分支
            </div>
          </div>
          <div class="git-branch-create">
            <input
              v-model="newBranchName"
              class="git-branch-input"
              type="text"
              placeholder="新建分支…"
              :disabled="branchBusy"
              @keydown.enter="createBranch()"
            />
            <button
              class="git-branch-create-btn"
              :disabled="branchBusy || !newBranchName.trim()"
              @click="createBranch()"
            >
              新建
            </button>
          </div>
        </div>
      </div>

      <ul v-if="fileCount" class="git-file-list">
        <li
          v-for="file in gitStatus.files"
          :key="file.path"
          class="git-file"
          @click="openDiff(file)"
        >
          <span
            class="git-status-icon"
            :class="`git-status-${file.status}`"
            v-tooltip="gitStatusLabel(file.status)"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="gitStatusIcon(file.status)" />
            </svg>
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
