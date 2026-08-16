<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import ContextMenu from "./ContextMenu.vue";
import ModalDialog from "./ModalDialog.vue";
import GitCommitBar from "./GitCommitBar.vue";
import GitFileTree from "./GitFileTree.vue";
import GitBranchMenu from "./GitBranchMenu.vue";
import GitHistoryList from "./GitHistoryList.vue";
import { setToast, toastError, workspace } from "../composables/useCodex";
import { useActionMenu } from "../composables/useActionMenu";
import { useGitRemoteOps } from "../composables/useGitRemoteOps";
import {
  useGitFileActions,
  type GitSection,
} from "../composables/useGitFileActions";
import {
  gitErrorMsg,
  gitInitBusy,
  gitRevealTarget,
  gitState,
  gitStatus,
  initGitRepo,
  refreshGitChanges,
  setGitChangesActive,
} from "../composables/useGitChanges";
import { ensureEntryIcons, iconFor } from "../composables/useSessionFs";
import { openDiffTab } from "../composables/useEditorTabs";
import { joinFsPath, type FsEntry } from "../lib/sessionFs";
import {
  normalizeDiffKind,
  type GitFile,
  type GitStatus,
} from "../lib/gitChanges";
import {
  buildGitTree,
  flattenRows,
  type GitDirNode,
  type GitFileNode,
  type GitTreeNode,
} from "../lib/gitTree";
import {
  ICON_ARROW_DOWN,
  ICON_ARROW_RIGHT,
  ICON_PLUS,
} from "../lib/icons";

const props = defineProps<{ active: boolean }>();

const {
  gitAvailable,
  pullBusy,
  pushBusy,
  branchLabel,
  repoWorkspace,
  doPull,
  doPush,
  checkGitAvailable,
} = useGitRemoteOps({ gitStatus });
/** 当前在 Git 面板中高亮的变更文件（相对仓库根路径） */
const selectedGitPath = ref("");

watch(
  () => props.active,
  (v) => {
    setGitChangesActive(v);
    if (v) void checkGitAvailable();
    // 面板切换进入时无条件重试：diff 标签激活早于 git 状态加载也能定位
    if (v) attemptReveal();
  },
  { immediate: true },
);

const confirmInit = ref(false);

const branchMenuOpen = ref(false);
/** 分支弹层忙碌态（由 GitBranchMenu v-model 同步，用于头部按钮禁用） */
const branchBusy = ref(false);

const ICON_ARROW_UP =
  "M4 12l1.41 1.41L11 7.83V20h2V7.83l5.58 5.59L20 12l-8-8-8 8z";

const {
  ctxMenu,
  openCtx,
  onWindowClick: onMenuWindowClick,
  onWindowScroll: onMenuWindowScroll,
  onKeydown: onMenuKeydown,
} = useActionMenu({ width: 190, scrollScope: ".git-view" });
const {
  gitActionBusy,
  openFileCtx,
  openDirCtx,
  stageAll,
  unstageAll,
} = useGitFileActions({ gitStatus, openCtx, openDiff });

/** 手动折叠的目录集合；未记录 = 默认展开，折叠状态跨刷新保留 */
const collapsedDirs = reactive(new Set<string>());

/** 可折叠分区 key：changes | staged | history；未记录 = 默认展开。
 * 历史记录默认折叠（内容较多），更改/暂存区默认展开。折叠状态跨刷新保留。 */
const collapsedSections = reactive(new Set<string>(["history"]));

function isSectionCollapsed(key: string): boolean {
  return collapsedSections.has(key);
}

function toggleSection(key: string) {
  if (collapsedSections.has(key)) collapsedSections.delete(key);
  else collapsedSections.add(key);
}

const worktreeFiles = computed(() =>
  gitStatus.value ? gitStatus.value.files.filter((f) => f.worktree) : [],
);
const stagedFiles = computed(() =>
  gitStatus.value ? gitStatus.value.files.filter((f) => f.staged) : [],
);
const worktreeRows = computed(() =>
  flattenRows(buildGitTree(worktreeFiles.value, collapsedDirs)),
);
const stagedRows = computed(() =>
  flattenRows(buildGitTree(stagedFiles.value, collapsedDirs)),
);
const stagedCount = computed(() => stagedFiles.value.length);
/** 更改总数（每个文件计一次，含已暂存 + 工作区 + 未跟踪） */
const changeCount = computed(() => gitStatus.value?.files.length ?? 0);

/** 归一化目标为相对仓库根路径；仓库不匹配返回 null */
function normGitRel(target: { workspace: string; path: string }): string | null {
  const root = repoWorkspace.value;
  if (!root) return null;
  if (target.workspace && target.workspace !== root) return null;
  const rootNorm = root.replace(/\\/g, "/").replace(/\/+$/, "");
  let p = target.path.replace(/\\/g, "/");
  if (p.startsWith(rootNorm + "/")) {
    p = p.slice(rootNorm.length + 1);
  }
  return p || null;
}

/** 在 Git 面板中定位并高亮目标变更文件：展开祖先目录、滚动到可见 */
function attemptReveal() {
  const target = gitRevealTarget.value;
  if (!target) return;
  const rel = normGitRel(target);
  if (rel === null) return;
  // 在完整文件列表中查找（折叠的祖先目录不影响定位），变更区优先、暂存区次之
  const match =
    worktreeFiles.value.find((f) => f.path === rel) ??
    stagedFiles.value.find((f) => f.path === rel);
  if (!match) return;
  const parts = rel.split("/");
  let acc = "";
  for (let i = 0; i < parts.length - 1; i++) {
    acc = acc ? `${acc}/${parts[i]}` : parts[i];
    collapsedDirs.delete(acc);
  }
  selectedGitPath.value = rel;
  void nextTick(() => {
    document
      .querySelector<HTMLElement>(`[data-git-path="${CSS.escape(rel)}"]`)
      ?.scrollIntoView({ block: "center" });
  });
}

// diff 标签激活请求 / git 状态晚加载：定位并高亮对应变更行
watch([gitRevealTarget, gitStatus], () => attemptReveal());

/** 变更文件 → FsEntry（供系统文件图标管线复用；repoWorkspace 为空时回退相对路径） */
function gitFileEntry(file: GitFile): FsEntry {
  const root = repoWorkspace.value;
  return {
    name: file.path.split("/").pop() ?? file.path,
    path: root ? joinFsPath(root, file.path) : file.path,
    relPath: file.path,
    isDir: false,
    size: null,
    modifiedAtMs: 0,
    createdAtMs: 0,
    childCount: null,
  };
}

/** 文件类型图标：共享图标缓存取不到时返回 undefined，渲染层回退 SVG */
function gitFileIcon(file: GitFile): string | undefined {
  return iconFor(gitFileEntry(file)) ?? undefined;
}

// 变更列表可见行变化时懒加载缺失的文件图标（与资源面板同管线）
watch(
  [worktreeRows, stagedRows, repoWorkspace],
  () => {
    const root = repoWorkspace.value;
    if (!root) return;
    const files = [
      ...worktreeRows.value,
      ...stagedRows.value,
    ]
      .filter((r): r is GitFileNode => r.kind === "file")
      .map((r) => gitFileEntry(r.file));
    if (files.length) void ensureEntryIcons(files, root);
  },
);

function startInit() {
  confirmInit.value = true;
}

async function doInit() {
  confirmInit.value = false;
  await initGitRepo();
}

/** 分支弹层开关：打开后由 GitBranchMenu 挂载时自行加载列表与远端 */
async function toggleBranchMenu() {
  branchMenuOpen.value = !branchMenuOpen.value;
}

/** 分支/远端操作导致仓库状态变化：回写 gitStatus（联动提交历史刷新） */
function applyBranchStatus(st: GitStatus) {
  gitStatus.value = st;
}

function onWindowClick(e: MouseEvent) {
  // 右键菜单：任意外部 click 关闭（弹层内部点击由各自处理器负责）
  if (e.target instanceof Element && e.target.closest(".ctx-menu")) return;
  onMenuWindowClick();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  onMenuKeydown(e);
}

function onWindowScroll(e: Event) {
  // 右键菜单：仅面板自身滚动时关闭；聊天区等外部滚动不影响
  if (!(e.target instanceof Element)) return;
  if (!e.target.closest(".git-view")) return;
  onMenuWindowScroll(e);
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
  const root = gitStatus.value?.repoWorkspace;
  if (!root) return;
  try {
    const diff = await invoke<string>("git_changes_diff", {
      workspace: root,
      path: file.path,
      kind: normalizeDiffKind(file.status),
    });
    if (!diff) {
      setToast("该文件无内容变化（可能仅为重命名）");
      return;
    }
    await openDiffTab({
      path: file.path,
      kind: normalizeDiffKind(file.status),
      diff,
      workspace: root,
    });
  } catch (e) {
    setToast(toastError(e));
  }
}

/** GitFileTree 行单击：目录折叠/展开，文件打开 diff */
function onFileRowClick(row: GitTreeNode) {
  if (row.kind === "dir") toggleDirRow(row);
  else void openDiff(row.file);
}

/** GitFileTree 行右键：目录/文件菜单（带分区参数） */
function onFileRowContext(
  section: GitSection,
  row: GitTreeNode,
  e: MouseEvent,
) {
  if (row.kind === "dir") openDirCtx(section, row, e);
  else openFileCtx(section, row.file, e);
}

function toggleDirRow(node: GitDirNode) {
  if (collapsedDirs.has(node.relPath)) collapsedDirs.delete(node.relPath);
  else collapsedDirs.add(node.relPath);
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
        class="btn primary git-init-btn"
        :disabled="gitInitBusy"
        @click="startInit"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path :d="ICON_PLUS" />
        </svg>
        <span>{{ gitInitBusy ? "初始化中…" : "添加到 Git" }}</span>
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
          v-tooltip="repoWorkspace"
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
        <div class="git-pull-push">
          <button
            class="git-icon-btn git-pull"
            :class="{ busy: pullBusy }"
            :disabled="
              !gitStatus?.hasRemote ||
              !gitAvailable ||
              pullBusy ||
              pushBusy ||
              branchLabel === 'HEAD'
            "
            :aria-label="pullBusy ? '拉取中…' : '拉取'"
            v-tooltip="
              !gitStatus?.hasRemote
                ? '未配置远端，无法拉取'
                : !gitAvailable
                ? '未检测到 git，无法拉取'
                : branchLabel === 'HEAD'
                  ? '游离 HEAD 无法拉取'
                  : pullBusy
                    ? '拉取中…'
                    : pushBusy
                      ? '推送中…'
                      : '拉取'
            "
            @click="doPull()"
          >
            <svg v-if="!pullBusy" viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z"
              />
            </svg>
            <span v-else class="git-pull-text">拉取中…</span>
          </button>
          <span class="git-pull-push-divider" aria-hidden="true"></span>
          <button
            class="git-icon-btn git-push"
            :class="{ busy: pushBusy }"
            :disabled="
              !gitStatus?.hasRemote ||
              !gitAvailable ||
              pushBusy ||
              pullBusy ||
              branchLabel === 'HEAD'
            "
            :aria-label="pushBusy ? '推送中…' : '推送'"
            v-tooltip="
              !gitStatus?.hasRemote
                ? '未配置远端，无法推送'
                : !gitAvailable
                ? '未检测到 git，无法推送'
                : branchLabel === 'HEAD'
                  ? '游离 HEAD 无法推送'
                  : pushBusy
                    ? '推送中…'
                    : pullBusy
                      ? '拉取中…'
                      : '推送'
            "
            @click="doPush()"
          >
            <svg v-if="!pushBusy" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M9 16h6v-6h4l-7-7-7 7h4v6zm-4 2h14v2H5v-2z" />
            </svg>
            <span v-else class="git-push-text">推送中…</span>
          </button>
        </div>
        <!-- 分支弹层作为 .git-head 子节点，absolute 定位相对头部，避免被面板 overflow 裁掉 -->
        <GitBranchMenu
          v-if="branchMenuOpen"
          :workspace="repoWorkspace"
          :branch-label="branchLabel"
          v-model:busy="branchBusy"
          @close="branchMenuOpen = false"
          @status="applyBranchStatus"
        />
      </div>

      <div class="git-section" :class="{ collapsed: isSectionCollapsed('changes') }">
        <div
          class="git-section-head"
          role="button"
          tabindex="0"
          :aria-expanded="!isSectionCollapsed('changes')"
          @click="toggleSection('changes')"
          @keydown.enter="toggleSection('changes')"
        >
          <svg class="git-section-arrow" viewBox="0 0 24 24" aria-hidden="true">
            <path
              :d="
                isSectionCollapsed('changes')
                  ? ICON_ARROW_RIGHT
                  : ICON_ARROW_DOWN
              "
            />
          </svg>
          <span>更改</span>
          <span class="git-section-actions">
            <span v-if="changeCount > 0" class="git-section-count">{{
              changeCount
            }}</span>
            <button
              class="git-icon-btn git-section-action git-section-stage"
              aria-label="全部暂存"
              v-tooltip="'全部暂存'"
              :disabled="gitActionBusy || !worktreeRows.length"
              @click.stop="stageAll()"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_ARROW_DOWN" />
              </svg>
            </button>
          </span>
        </div>
        <template v-if="!isSectionCollapsed('changes')">
          <GitFileTree
            :rows="worktreeRows"
            section="changes"
            :selected-path="selectedGitPath"
            :file-icon="gitFileIcon"
            empty-text="无更改"
            @row-click="onFileRowClick"
            @row-context="onFileRowContext"
          />
        </template>
      </div>

      <div class="git-section" :class="{ collapsed: isSectionCollapsed('staged') }">
        <div
          class="git-section-head"
          role="button"
          tabindex="0"
          :aria-expanded="!isSectionCollapsed('staged')"
          @click="toggleSection('staged')"
          @keydown.enter="toggleSection('staged')"
        >
          <svg class="git-section-arrow" viewBox="0 0 24 24" aria-hidden="true">
            <path
              :d="
                isSectionCollapsed('staged')
                  ? ICON_ARROW_RIGHT
                  : ICON_ARROW_DOWN
              "
            />
          </svg>
          <span>暂存更改</span>
          <span class="git-section-actions">
            <span v-if="stagedCount > 0" class="git-section-count">{{
              stagedCount
            }}</span>
            <button
              class="git-icon-btn git-section-action git-section-unstage"
              aria-label="全部取消暂存"
              v-tooltip="'全部取消暂存'"
              :disabled="gitActionBusy || !stagedRows.length"
              @click.stop="unstageAll()"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_ARROW_UP" />
              </svg>
            </button>
          </span>
        </div>
        <template v-if="!isSectionCollapsed('staged')">
          <GitCommitBar
            :workspace="repoWorkspace"
            :staged-count="stagedCount"
            @committed="applyBranchStatus"
          />
          <GitFileTree
            :rows="stagedRows"
            section="staged"
            :selected-path="selectedGitPath"
            :file-icon="gitFileIcon"
            empty-text="无暂存更改"
            @row-click="onFileRowClick"
            @row-context="onFileRowContext"
          />
        </template>
      </div>

      <div class="git-section" :class="{ collapsed: isSectionCollapsed('history') }">
        <div
          class="git-section-head"
          role="button"
          tabindex="0"
          :aria-expanded="!isSectionCollapsed('history')"
          @click="toggleSection('history')"
          @keydown.enter="toggleSection('history')"
        >
          <svg class="git-section-arrow" viewBox="0 0 24 24" aria-hidden="true">
            <path
              :d="
                isSectionCollapsed('history')
                  ? ICON_ARROW_RIGHT
                  : ICON_ARROW_DOWN
              "
            />
          </svg>
          <span>提交历史</span>
        </div>
        <template v-if="!isSectionCollapsed('history')">
          <GitHistoryList :workspace="repoWorkspace" :reload-key="gitStatus" />
        </template>
      </div>
    </template>

    <ContextMenu
      v-if="ctxMenu"
      :items="ctxMenu.items"
      :x="ctxMenu.x"
      :y="ctxMenu.y"
      :disabled="gitActionBusy"
      @close="ctxMenu = null"
    />

    <ModalDialog
      v-if="confirmInit"
      title="添加到 Git"
      closable
      mask-close
      @close="confirmInit = false"
    >
      确定要将当前目录初始化为 Git 仓库吗？<br />
      <span class="git-confirm-path">
        {{ workspace || "当前工作目录" }}
      </span>
      <p class="git-confirm-desc">
        将执行 git init，仅初始化、不会自动提交；初始化后现有文件会以“未跟踪”状态显示。
      </p>
      <template #foot>
        <button class="btn" @click="confirmInit = false">取消</button>
        <button class="btn git-init-ok" :disabled="gitInitBusy" @click="doInit">
          {{ gitInitBusy ? "初始化中…" : "确认" }}
        </button>
      </template>
    </ModalDialog>
  </div>
</template>
