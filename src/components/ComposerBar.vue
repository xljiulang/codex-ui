<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import MentionMenu from "./MentionMenu.vue";
import ModelMenu from "./ModelMenu.vue";
import PermissionMenu from "./PermissionMenu.vue";
import PlusMenu from "./PlusMenu.vue";
import TaskModeMenu from "./TaskModeMenu.vue";
import {
  interrupt,
  effectiveEffort,
  modelDisplayName,
  permissionChip,
  sendPrompt,
  store,
} from "../composables/useCodex";
import type { UserInput } from "../lib/types";
import {
  baseName,
  matchMentionToken,
  toUserAttachment,
  type FuzzyFileResult,
} from "../lib/mention";

const text = ref("");
const mention = ref<null | { kind: "@" | "$"; token: string; start: number }>(
  null,
);
const inputEl = ref<HTMLTextAreaElement | null>(null);
const mentionMenu = ref<InstanceType<typeof MentionMenu> | null>(null);

// @ 文件引用：模糊搜索结果
const fileResults = ref<FuzzyFileResult[]>([]);
const searchingFiles = ref(false);
let searchSeq = 0;
let searchTimer: number | undefined;

// 用户输入历史（仅内存），供向上/向下键选择，行为类似 Linux shell
const sentHistory: string[] = [];
let historyIndex = -1;

function onInput() {
  mention.value = matchMentionToken(text.value);
  if (!mention.value) {
    fileResults.value = [];
    searchingFiles.value = false;
  }
}

function scheduleFileSearch(token: string) {
  if (searchTimer) window.clearTimeout(searchTimer);
  if (!token) {
    fileResults.value = [];
    searchingFiles.value = false;
    searchSeq++;
    return;
  }
  searchTimer = window.setTimeout(() => {
    void runFileSearch(token);
  }, 250);
}

async function runFileSearch(token: string) {
  const seq = ++searchSeq;
  const root = mentionRoot();
  if (!root) {
    searchingFiles.value = false;
    return;
  }
  searchingFiles.value = true;
  try {
    const res = await invoke<{ files?: FuzzyFileResult[] }>("codex_rpc", {
      method: "fuzzyFileSearch",
      params: { query: token, roots: [root], cancellationToken: null },
    });
    if (seq !== searchSeq) return;
    fileResults.value = (res.files ?? []).slice(0, 50);
  } catch (e) {
    if (seq === searchSeq) {
      fileResults.value = [];
      store.toast = String(e);
    }
  } finally {
    if (seq === searchSeq) searchingFiles.value = false;
  }
}

watch(
  () => mention.value,
  (m) => {
    if (m?.kind === "@") scheduleFileSearch(m.token);
    else if (searchTimer) window.clearTimeout(searchTimer);
  },
);

function removeMentionToken() {
  const m = mention.value;
  if (!m) return;
  text.value =
    text.value.slice(0, m.start) + text.value.slice(m.start + 1 + m.token.length);
  mention.value = null;
}

function refocusInput() {
  void nextTick(() => inputEl.value?.focus());
}

function onSelectAttachment(a: UserInput) {
  removeMentionToken();
  store.attachments.push(a);
  refocusInput();
}

function mentionRoot(): string {
  return (
    store.newChatCwd ?? store.currentThreadCwd ?? store.server.workspace ?? ""
  );
}

function onPickFiles() {
  removeMentionToken();
  void (async () => {
    try {
      const files = await invoke<string[]>("pick_files", {
        multiple: true,
        initialDir: mentionRoot(),
      });
      for (const f of files) {
        const a = toUserAttachment(baseName(f), f);
        store.attachments.push(a);
      }
    } catch (e) {
      store.toast = String(e);
    } finally {
      refocusInput();
    }
  })();
}

function onPickDir() {
  removeMentionToken();
  void (async () => {
    try {
      const dir = await invoke<string | null>("pick_directory", {
        initialDir: mentionRoot(),
      });
      if (dir) {
        const a = toUserAttachment(baseName(dir), dir);
        store.attachments.push(a);
      }
    } catch (e) {
      store.toast = String(e);
    } finally {
      refocusInput();
    }
  })();
}

function closeMenus() {
  store.plusOpen = false;
  store.permOpen = false;
  store.taskOpen = false;
  store.modelOpen = false;
  mention.value = null;
}

function onKeydownGlobal(e: KeyboardEvent) {
  if (e.key === "Escape") closeMenus();
}

onMounted(() => {
  window.addEventListener("keydown", onKeydownGlobal);
  inputEl.value?.focus();
});
onBeforeUnmount(() => window.removeEventListener("keydown", onKeydownGlobal));

// 新建对话后输入框重新获得焦点（组件未卸载的情况，如聊天页直接点“新建对话”）
watch(
  () => store.currentThreadId,
  (v) => {
    if (!v) inputEl.value?.focus();
  },
);

function onKeydown(e: KeyboardEvent) {
  // 输入法组合中（如中文拼音选字）的按键不触发提交/历史选择
  if (e.isComposing || e.keyCode === 229) return;
  // @ 文件引用菜单打开时：Enter 选中高亮项，↑↓ 移动高亮
  if (
    mention.value?.kind === "@" &&
    (e.key === "Enter" || e.key === "ArrowUp" || e.key === "ArrowDown")
  ) {
    e.preventDefault();
    if (e.key === "Enter") {
      mentionMenu.value?.selectHighlighted();
    } else {
      mentionMenu.value?.move(e.key === "ArrowUp" ? -1 : 1);
    }
    return;
  }
  if (e.key === "Enter") {
    // Enter 快捷发送：开启时 Enter 发送 / Shift+Enter 换行；
    // 关闭时 Enter 换行 / Ctrl+Enter 发送
    const shouldSend = store.settings.enter_to_send ? !e.shiftKey : e.ctrlKey;
    if (shouldSend) {
      e.preventDefault();
      submit(e.ctrlKey);
    }
    return;
  }
  if (e.key === "ArrowUp" || e.key === "ArrowDown") {
    const ta = e.target as HTMLTextAreaElement;
    // 仅当光标位于第一行时触发历史选择，否则保留默认的上下移动光标
    const firstLineEnd = ta.value.indexOf("\n");
    const firstLineLen = firstLineEnd === -1 ? ta.value.length : firstLineEnd;
    if (ta.selectionStart > firstLineLen) return;
    e.preventDefault();
    if (e.key === "ArrowUp") {
      if (!sentHistory.length) return;
      if (historyIndex === -1) historyIndex = sentHistory.length - 1;
      else if (historyIndex > 0) historyIndex--;
      text.value = sentHistory[historyIndex];
    } else {
      if (historyIndex === -1) return;
      historyIndex++;
      if (historyIndex >= sentHistory.length) {
        historyIndex = -1;
        text.value = "";
      } else {
        text.value = sentHistory[historyIndex];
      }
    }
  }
}

function submit(flip = false) {
  closeMenus(); // 发送后关闭可能开着的菜单，避免回合中还能切换模式
  const t = text.value;
  text.value = "";
  if (t.trim()) {
    sentHistory.push(t);
    if (sentHistory.length > 100) sentHistory.shift();
  }
  historyIndex = -1;
  void sendPrompt(t, flip);
}

function removeAttachment(i: number) {
  store.attachments.splice(i, 1);
}

function imageSrc(path: string): string {
  try {
    return convertFileSrc(path);
  } catch {
    return path;
  }
}

function attachmentLabel(a: UserInput): string {
  if (a.type === "mention") return `@${a.name}`;
  if (a.type === "skill") return `$${a.name}`;
  if (a.type === "localImage") return a.path.split(/[\\/]/).pop() ?? a.path;
  return a.text;
}

const newChatCwdLabel = computed(() => store.newChatCwd ?? store.server.workspace);

// 上下文窗口使用情况：window 未知时不显示
const ctxUsage = computed(() => {
  const u = store.threadTokenUsage;
  if (!u || u.window == null || u.window <= 0) return null;
  return {
    pct: Math.min(100, Math.round((u.used / u.window) * 100)),
    used: u.used,
    window: u.window,
  };
});

const ctxTooltip = computed(() => {
  const u = ctxUsage.value;
  if (!u) return "";
  return `上下文已用 ${formatTokens(u.used)}，共 ${formatTokens(u.window)}`;
});

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return String(n);
}

async function pickNewChatCwd() {
  try {
    const dir = await invoke<string | null>("pick_directory");
    if (dir) store.newChatCwd = dir;
  } catch (e) {
    store.toast = String(e);
  }
}

function modelChipLabel(): string {
  const name = modelDisplayName(store.model);
  const effort = effectiveEffort();
  return effort ? `${name}(${effort})` : name;
}

function taskModeLabel(): string {
  if (store.taskMode === "plan") return "计划模式";
  if (store.taskMode === "goal") return "目标模式";
  return "执行模式";
}

function openGoalDialog() {
  store.goalOpen = true;
}
</script>

<template>
  <div class="composer">
    <div v-if="!store.currentThreadId" class="newchat-cwd-row">
      <svg viewBox="0 0 24 24">
        <path
          d="M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"
        />
      </svg>
      <span class="newchat-cwd-label">项目目录</span>
      <button
        class="newchat-cwd-value"
        :title="newChatCwdLabel"
        @click="pickNewChatCwd()"
      >
        {{ newChatCwdLabel }}
      </button>
      <button
        v-if="store.newChatCwd"
        class="newchat-cwd-reset"
        title="恢复默认工作目录"
        @click="store.newChatCwd = null"
      >
        ×
      </button>
    </div>
    <div class="composer-input-row">
      <div class="menu-anchor input-anchor">
        <textarea
          ref="inputEl"
          v-model="text"
          rows="2"
          placeholder="输入消息，@ 引用文件 / $ 调用技能…"
          @input="onInput"
          @keydown="onKeydown"
        ></textarea>
        <MentionMenu
          ref="mentionMenu"
          v-if="mention"
          :kind="mention.kind"
          :token="mention.token"
          :results="fileResults"
          :searching="searchingFiles"
          @close="mention = null"
          @pick-files="onPickFiles()"
          @pick-dir="onPickDir()"
          @select-attachment="onSelectAttachment($event)"
        />
      </div>
      <div class="composer-left">
        <div class="menu-anchor">
          <button class="plus-btn" title="添加内容" @click="store.plusOpen = !store.plusOpen">
            +
          </button>
          <PlusMenu v-if="store.plusOpen" @close="store.plusOpen = false" />
        </div>
        <div class="menu-anchor">
          <button
            class="perm-chip"
            title="权限模式"
            :disabled="store.turnActive"
            @click="store.permOpen = !store.permOpen"
          >
            <svg v-if="store.permissionMode === 'full-access'" viewBox="0 0 24 24">
              <path
                d="M12 2 4 5v6c0 5.55 3.84 10.74 8 12 4.16-1.26 8-6.45 8-12V5l-8-3zm1 14h-2v-2h2v2zm0-4h-2V8h2v4z"
              />
            </svg>
            {{ permissionChip() }}
            <svg class="chevron" viewBox="0 0 16 16">
              <path d="M4 6l4 4 4-4z" />
            </svg>
          </button>
          <PermissionMenu v-if="store.permOpen" @close="store.permOpen = false" />
        </div>
        <div class="menu-anchor">
          <button
            class="task-chip"
            title="任务模式"
            :disabled="store.turnActive"
            @click="store.taskOpen = !store.taskOpen"
          >
            {{ taskModeLabel() }}
            <svg viewBox="0 0 16 16">
              <path d="M4 6l4 4 4-4z" />
            </svg>
          </button>
          <TaskModeMenu v-if="store.taskOpen" @close="store.taskOpen = false" />
        </div>
        <div v-if="store.goalText" class="menu-anchor">
          <button
            class="goal-chip"
            title="目标"
            @click="openGoalDialog()"
          >
            目标
          </button>
        </div>
      </div>
      <div class="composer-right">
        <span
          v-if="ctxUsage"
          class="ctx-window"
          :title="ctxTooltip"
        >
          {{ ctxUsage.pct }}%
        </span>
        <div class="menu-anchor">
          <button class="model-chip" title="模型" @click="store.modelOpen = !store.modelOpen">
          {{ modelChipLabel() }}
            <svg viewBox="0 0 16 16">
              <path d="M4 6l4 4 4-4z" />
            </svg>
          </button>
          <ModelMenu v-if="store.modelOpen" @close="store.modelOpen = false" />
        </div>
        <button
          v-if="store.turnActive"
          class="send-btn stop"
          title="停止生成"
          @click="interrupt()"
        >
          <svg viewBox="0 0 24 24">
            <rect x="6" y="6" width="12" height="12" rx="1.5" />
          </svg>
          停止
        </button>
        <button
          v-else
          class="send-btn"
          title="发送"
          :class="{ lit: !!(text.trim() || store.attachments.length) }"
          :disabled="!text.trim() && store.attachments.length === 0"
          @click="submit()"
        >
          <svg viewBox="0 0 24 24">
            <path d="M12 19V5M5 12l7-7 7 7" />
          </svg>
          发送
        </button>
      </div>
    </div>
    <div v-if="store.attachments.length" class="attachment-row">
      <span v-for="(a, i) in store.attachments" :key="i" class="attachment-chip">
        <img
          v-if="a.type === 'localImage'"
          class="attachment-thumb"
          :src="imageSrc(a.path)"
          alt=""
        />
        {{ attachmentLabel(a) }}
        <button title="移除" @click="removeAttachment(i)">×</button>
      </span>
    </div>

    <div
      v-if="
        store.plusOpen ||
        store.permOpen ||
        store.taskOpen ||
        store.modelOpen ||
        mention
      "
      class="menu-backdrop"
      @click="closeMenus()"
    ></div>
  </div>
</template>
