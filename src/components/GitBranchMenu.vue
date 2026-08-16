<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import GitRemotesSection from "./GitRemotesSection.vue";
import { askConfirm, setToast, toastError } from "../composables/useCodex";
import {
  type GitBranches,
  type GitMergeResult,
  type GitStatus,
} from "../lib/gitChanges";
import {
  ICON_CLOSE,
  ICON_PLUS,
  ICON_REFRESH,
} from "../lib/icons";

const ICON_MERGE =
  "M17 20.41L18.41 19 15 15.59 13.59 17 17 20.41zM7.5 8H11v5.59L5.59 19 7 20.41l6-6V8h3.5L12 3.5 7.5 8z";

const props = defineProps<{
  /** 仓库根目录（invoke 参数与分支操作的工作区） */
  workspace: string;
  /** 当前分支名（驱动高亮与合并/删除按钮显隐） */
  branchLabel: string;
  /** 分支操作忙碌态（v-model 同步到头部按钮禁用） */
  busy: boolean;
}>();

const emit = defineEmits<{
  close: [];
  "update:busy": [value: boolean];
  /** 分支/远端操作导致仓库状态变化（GitView 回写 gitStatus 并联动提交历史） */
  status: [status: GitStatus];
}>();

const branchBusy = ref(false);
const branches = ref<string[]>([]);
const remoteBranches = ref<string[]>([]);
const currentUpstream = ref<string | null>(null);
const newBranchName = ref("");
const fetchBusy = ref(false);
const mergeBusy = ref(false);
const currentRemote = ref<string | null>(null);

function setBranchBusy(v: boolean) {
  branchBusy.value = v;
  emit("update:busy", v);
}

/** 分支列表统一回填（兼容旧返回：缺失字段取默认值） */
function applyBranches(res: GitBranches) {
  branches.value = res.branches ?? [];
  remoteBranches.value = res.remoteBranches ?? [];
  currentUpstream.value = res.currentUpstream ?? null;
}

/** 挂载即加载：分支列表与远端管理分区并行拉取（弹层由 GitView v-if 控制） */
onMounted(() => {
  void openBranchMenu();
});

async function openBranchMenu() {
  setBranchBusy(true);
  try {
    const res = await invoke<GitBranches>("git_changes_branches", {
      workspace: props.workspace,
    });
    applyBranches(res);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    setBranchBusy(false);
  }
}

/** 「拉取刷新」目标远端：优先当前上游所在远端，其次远端管理状态，最后为空串（后端取默认） */
function defaultFetchRemote(): string {
  const upstream = currentUpstream.value;
  if (upstream) {
    const idx = upstream.indexOf("/");
    if (idx > 0) return upstream.slice(0, idx);
  }
  return currentRemote.value ?? "";
}

/** 拉取远端更新并刷新分支列表（含远程分支） */
async function fetchRemoteBranches() {
  if (fetchBusy.value || branchBusy.value) return;
  fetchBusy.value = true;
  try {
    const res = await invoke<GitBranches>("git_changes_remote_fetch", {
      workspace: props.workspace,
      remote: defaultFetchRemote(),
    });
    applyBranches(res);
    setToast("已拉取远端更新");
  } catch (e) {
    setToast(toastError(e));
  } finally {
    fetchBusy.value = false;
  }
}

/** 远程分支短名（去掉远端前缀，如 origin/dev → dev） */
function remoteBranchShortName(remoteBranch: string): string {
  const idx = remoteBranch.indexOf("/");
  return idx >= 0 ? remoteBranch.slice(idx + 1) : remoteBranch;
}

/** 点击远程分支：本地已有同名分支走本地切换，否则检出为本地跟踪分支 */
async function checkoutRemoteBranch(remoteBranch: string) {
  if (branchBusy.value) return;
  const shortName = remoteBranchShortName(remoteBranch);
  if (shortName === props.branchLabel) return;
  setBranchBusy(true);
  try {
    if (branches.value.includes(shortName)) {
      const st = await invoke<GitStatus>("git_changes_branch_switch", {
        workspace: props.workspace,
        name: shortName,
      });
      emit("status", st);
      emit("close");
      setToast(`已切换到本地分支 ${shortName}`);
      return;
    }
    const st = await invoke<GitStatus>("git_changes_branch_checkout_remote", {
      workspace: props.workspace,
      remoteBranch,
    });
    emit("status", st);
    emit("close");
    setToast(`已检出远程分支 ${remoteBranch}`);
    // 刷新分支列表（新增本地分支与上游）
    const res = await invoke<GitBranches>("git_changes_branches", {
      workspace: props.workspace,
    });
    applyBranches(res);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    setBranchBusy(false);
  }
}

/** 删除远程分支：二次确认（当前上游时追加警告），成功后刷新列表 */
async function deleteRemoteBranch(remoteBranch: string) {
  const isUpstream = remoteBranch === currentUpstream.value;
  const ok = await askConfirm({
    title: "删除远程分支",
    message: isUpstream
      ? `确定删除远程分支「${remoteBranch}」吗？当前分支跟踪该远程分支，删除后需重新设置上游。`
      : `确定删除远程分支「${remoteBranch}」吗？远端上的该分支将被移除。`,
    confirmLabel: "删除远程分支",
  });
  if (!ok || branchBusy.value) return;
  setBranchBusy(true);
  try {
    const res = await invoke<GitBranches>("git_changes_remote_branch_delete", {
      workspace: props.workspace,
      remoteBranch,
    });
    applyBranches(res);
    setToast(`已删除远程分支 ${remoteBranch}`);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    setBranchBusy(false);
  }
}

async function switchBranch(name: string) {
  if (branchBusy.value || name === props.branchLabel) return;
  setBranchBusy(true);
  try {
    const st = await invoke<GitStatus>("git_changes_branch_switch", {
      path: props.workspace,
      name,
    });
    emit("status", st);
    emit("close");
  } catch (e) {
    setToast(toastError(e));
  } finally {
    setBranchBusy(false);
  }
}

async function createBranch() {
  const name = newBranchName.value.trim();
  if (!name || branchBusy.value) return;
  setBranchBusy(true);
  try {
    const st = await invoke<GitStatus>("git_changes_branch_create", {
      path: props.workspace,
      name,
    });
    emit("status", st);
    newBranchName.value = "";
    // 重新拉取分支列表，保持弹层打开
    const res = await invoke<GitBranches>("git_changes_branches", {
      path: props.workspace,
    });
    applyBranches(res);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    setBranchBusy(false);
  }
}

async function deleteBranch(name: string) {
  if (branchBusy.value || name === props.branchLabel) return;
  setBranchBusy(true);
  try {
    const st = await invoke<GitStatus>("git_changes_branch_delete", {
      path: props.workspace,
      name,
    });
    emit("status", st);
    branches.value = branches.value.filter((b) => b !== name);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    setBranchBusy(false);
  }
}

/** 将分支合并到当前分支；成功后关闭弹层并提示结果 */
async function mergeBranch(name: string) {
  if (branchBusy.value || mergeBusy.value || name === props.branchLabel) return;
  mergeBusy.value = true;
  try {
    const res = await invoke<GitMergeResult>("git_changes_branch_merge", {
      path: props.workspace,
      name,
    });
    emit("status", res.status);
    setToast(res.message);
    emit("close");
  } catch (e) {
    setToast(toastError(e));
  } finally {
    mergeBusy.value = false;
  }
}

// ---------- 弹层关闭语义（与 GitView 原实现一致） ----------

function onWindowMousedown(e: MouseEvent) {
  // 仅外部 mousedown 关闭。WebView2 原生菜单「粘贴」只合成 click、不合成
  // mousedown，避免粘贴远端名/地址时弹层被误关。
  // 用 Element 而非 HTMLElement：点击 svg/path 等 SVG 目标也应正确判断
  if (!(e.target instanceof Element)) {
    emit("close");
    return;
  }
  if (
    e.target.closest(".git-branch-menu") ||
    e.target.closest(".git-branch-btn")
  ) {
    return;
  }
  emit("close");
}

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  emit("close");
}

function onWindowScroll(e: Event) {
  // 仅面板自身滚动时关闭；聊天区等外部滚动不影响弹层
  if (!(e.target instanceof Element)) return;
  // 弹层自身（含本地/远程/远端管理内部列表）滚动不关闭，粘贴/聚焦自动滚动不误关
  if (e.target.closest(".git-branch-menu")) return;
  if (!e.target.closest(".git-view")) return;
  emit("close");
}

onMounted(() => {
  window.addEventListener("mousedown", onWindowMousedown);
  window.addEventListener("keydown", onKeydown);
  window.addEventListener("scroll", onWindowScroll, true);
});
onBeforeUnmount(() => {
  window.removeEventListener("mousedown", onWindowMousedown);
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("scroll", onWindowScroll, true);
});
</script>

<template>
  <div class="git-branch-menu">
    <div class="git-branch-menu-list">
      <div class="git-branch-section-head">本地</div>
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
          class="git-branch-merge"
          v-tooltip="`将 ${b} 合并到 ${branchLabel}`"
          :disabled="branchBusy || mergeBusy"
          @click.stop="mergeBranch(b)"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_MERGE" />
          </svg>
        </button>
        <button
          v-if="b !== branchLabel"
          class="git-branch-delete"
          v-tooltip="'删除分支'"
          :disabled="branchBusy"
          @click.stop="deleteBranch(b)"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_CLOSE" />
          </svg>
        </button>
      </div>
      <div v-if="!branches.length" class="git-branch-menu-empty">
        暂无本地分支
      </div>
    </div>
    <div class="git-remote-branch-section">
      <div class="git-remote-branch-head">
        <span>远程</span>
        <button
          class="git-remote-branch-fetch"
          aria-label="拉取远端更新"
          v-tooltip="'拉取远端更新'"
          :disabled="fetchBusy"
          @click="fetchRemoteBranches()"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_REFRESH" />
          </svg>
          <span v-if="fetchBusy">拉取中…</span>
        </button>
      </div>
      <div v-if="remoteBranches.length" class="git-remote-branch-list">
        <div
          v-for="rb in remoteBranches"
          :key="rb"
          class="git-remote-branch-item"
          :class="{ upstream: rb === currentUpstream }"
          @click="checkoutRemoteBranch(rb)"
        >
          <span class="git-remote-branch-name">{{ rb }}</span>
          <span
            v-if="rb === currentUpstream"
            class="git-remote-branch-badge"
          >上游</span>
          <button
            class="git-remote-branch-delete"
            aria-label="删除远程分支"
            v-tooltip="'删除远程分支'"
            :disabled="branchBusy"
            @click.stop="deleteRemoteBranch(rb)"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_CLOSE" />
            </svg>
          </button>
        </div>
      </div>
      <div v-else class="git-remote-branch-empty">暂无远程分支</div>
    </div>
    <GitRemotesSection
      :workspace="workspace"
      v-model:current-remote="currentRemote"
    />
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
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path :d="ICON_PLUS" />
        </svg>
        <span>新建</span>
      </button>
    </div>
  </div>
</template>
