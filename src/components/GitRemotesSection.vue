<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { askConfirm, setToast, toastError } from "../composables/useCodex";
import type {
  GitRemote,
  GitRemotes,
} from "../lib/gitChanges";
import {
  ICON_DELETE,
  ICON_PLUS,
  ICON_SWITCH,
} from "../lib/icons";

const props = defineProps<{
  /** 仓库根目录（git_changes_remotes 系列命令的 workspace 参数） */
  workspace: string;
  /** 当前上游远端（v-model 同步到父级，供拉取刷新目标判定） */
  currentRemote: string | null;
}>();

const emit = defineEmits<{
  "update:currentRemote": [value: string | null];
}>();

const remotes = ref<GitRemote[]>([]);
const remoteBusy = ref(false);
const remoteLoading = ref(false);
const newRemoteName = ref("");
const newRemoteUrl = ref("");

onMounted(() => {
  void loadRemotes();
});

async function loadRemotes() {
  if (remoteLoading.value) return;
  remoteLoading.value = true;
  try {
    const res = await invoke<GitRemotes>("git_changes_remotes", {
      workspace: props.workspace,
    });
    remotes.value = res.remotes;
    emit("update:currentRemote", res.current);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    remoteLoading.value = false;
  }
}

async function addRemote() {
  if (remoteBusy.value) return;
  const name = newRemoteName.value.trim();
  const url = newRemoteUrl.value.trim();
  if (!name || !url) return;
  remoteBusy.value = true;
  try {
    const res = await invoke<GitRemotes>("git_changes_remote_add", {
      workspace: props.workspace,
      name,
      url,
    });
    remotes.value = res.remotes;
    emit("update:currentRemote", res.current);
    newRemoteName.value = "";
    newRemoteUrl.value = "";
    setToast(`已添加远端 ${name}`);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    remoteBusy.value = false;
  }
}

/** 把当前分支上游切换到目标远端（单上游替换）；已是当前上游时禁用 */
async function switchRemote(remote: GitRemote) {
  if (remoteBusy.value) return;
  if (remote.name === props.currentRemote) return;
  remoteBusy.value = true;
  try {
    const res = await invoke<GitRemotes>("git_changes_remote_switch_upstream", {
      workspace: props.workspace,
      remote: remote.name,
    });
    remotes.value = res.remotes;
    emit("update:currentRemote", res.current);
    setToast(`已将当前分支上游切换到 ${remote.name}`);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    remoteBusy.value = false;
  }
}

async function removeRemote(remote: GitRemote) {
  const isCurrent = remote.name === props.currentRemote;
  const ok = await askConfirm({
    title: "删除远端",
    message: isCurrent
      ? `确定删除远端「${remote.name}」吗？当前分支的拉取/推送依赖该远端，删除后需重新配置。`
      : `确定删除远端「${remote.name}」吗？`,
    confirmLabel: "删除远端",
  });
  if (!ok || remoteBusy.value) return;
  remoteBusy.value = true;
  try {
    const res = await invoke<GitRemotes>("git_changes_remote_remove", {
      workspace: props.workspace,
      name: remote.name,
    });
    remotes.value = res.remotes;
    emit("update:currentRemote", res.current);
    setToast(`已删除远端 ${remote.name}`);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    remoteBusy.value = false;
  }
}
</script>

<template>
  <div class="git-remote-branch-section git-remote-manage-section">
    <div class="git-remote-branch-head">
      <span>远端管理</span>
      <span v-if="remoteLoading" class="git-remote-loading">加载中…</span>
    </div>
    <div v-if="remotes.length" class="git-remote-list">
      <div v-for="r in remotes" :key="r.name" class="git-remote-row">
        <span class="git-remote-main">
          <span class="git-remote-name">
            {{ r.name }}
            <span
              v-if="r.name === currentRemote"
              class="git-remote-badge"
            >当前</span>
          </span>
          <span class="git-remote-url" v-tooltip="r.fetchUrl ?? undefined">
            {{ r.fetchUrl || "（未配置拉取地址）" }}
          </span>
          <span
            v-if="r.pushUrl && r.pushUrl !== r.fetchUrl"
            class="git-remote-url git-remote-push"
            v-tooltip="r.pushUrl"
          >
            推送：{{ r.pushUrl }}
          </span>
        </span>
        <span class="git-remote-actions">
          <button
            class="git-remote-switch"
            aria-label="切换远端"
            v-tooltip="
              r.name === currentRemote
                ? '当前远端'
                : '设为当前分支上游'
            "
            :disabled="remoteBusy || r.name === currentRemote"
            @click="switchRemote(r)"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_SWITCH" />
            </svg>
          </button>
          <button
            class="git-remote-delete"
            aria-label="删除远端"
            v-tooltip="'删除远端'"
            :disabled="remoteBusy"
            @click="removeRemote(r)"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_DELETE" />
            </svg>
          </button>
        </span>
      </div>
    </div>
    <div v-else-if="!remoteLoading" class="git-remote-branch-empty">
      暂无远端
    </div>
    <div class="git-remote-add">
      <input
        v-model="newRemoteName"
        class="git-remote-input"
        type="text"
        placeholder="远端名…"
        :disabled="remoteBusy"
        @keydown.enter="addRemote()"
      />
      <input
        v-model="newRemoteUrl"
        class="git-remote-input"
        type="text"
        placeholder="远端地址…"
        :disabled="remoteBusy"
        @keydown.enter="addRemote()"
      />
      <button
        class="btn sm git-remote-add-btn"
        :disabled="remoteBusy || !newRemoteName.trim() || !newRemoteUrl.trim()"
        @click="addRemote()"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path :d="ICON_PLUS" />
        </svg>
        <span>添加</span>
      </button>
    </div>
  </div>
</template>
