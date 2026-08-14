<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { EditorContent, useEditor } from "@tiptap/vue-3";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import MentionMenu from "./MentionMenu.vue";
import ModelMenu from "./ModelMenu.vue";
import PermissionMenu from "./PermissionMenu.vue";
import TaskModeMenu from "./TaskModeMenu.vue";
import GoalMenu from "./GoalMenu.vue";
import {
  clearGoal,
  interrupt,
  effectiveEffort,
  modelDisplayName,
  permissionChip,
  resolveCwd,
  sendPrompt,
  setToast,
  store,
  toastError,
} from "../composables/useCodex";
import type { UserInput } from "../lib/types";
import { debounce } from "../lib/debounce";
import {
  baseName,
  fileMentionSection,
  matchMentionToken,
  MY_REQUEST_MARKER,
  toUserAttachment,
  type FuzzyFileResult,
} from "../lib/mention";
import { permissionMode } from "../lib/permissions";
import { ICON_CHEVRON_DOWN, ICON_GOAL } from "../lib/icons";
import {
  Reference,
  docToRuns,
  refKindOfAttachment,
  runsToText,
  runsToWireText,
  tokenStartPos,
  type EditorRun,
} from "../lib/richEditor";
import { taskMode } from "../lib/tasks";

const mention = ref<null | { kind: "@" | "$"; token: string; start: number }>(
  null,
);
const mentionMenu = ref<InstanceType<typeof MentionMenu> | null>(null);

// @ 文件引用：模糊搜索结果
const fileResults = ref<FuzzyFileResult[]>([]);
const searchingFiles = ref(false);
let searchSeq = 0;
const debouncedFileSearch = debounce(
  (token: string) => void runFileSearch(token),
  250,
);

// 编辑器内联引用：chip id → 附件（仅插件/技能）；文件与图片走下方附件区
const refsById = ref(new Map<string, UserInput>());
const rowAttachments = ref<UserInput[]>([]);
const hasText = ref(false);

// 粘贴的截图/位图最大字节数（原路径文件不受限）
const MAX_PASTED_IMAGE_BYTES = 20 * 1024 * 1024;

// 输入框可拖拽高度：最低为现有自动高度，最高为窗口一半
const MIN_EDITOR_HEIGHT = 120;
const editorHeight = ref<number | null>(null);
const resizingEditor = ref(false);
const compacting = ref(false);
let resizeStartY = 0;
let resizeStartH = MIN_EDITOR_HEIGHT;

function maxEditorHeight(): number {
  return Math.max(MIN_EDITOR_HEIGHT, Math.round(window.innerHeight / 2));
}

function currentEditorHeight(): number {
  const el = document.querySelector<HTMLElement>(".rich-editor .ProseMirror");
  return el ? Math.round(el.getBoundingClientRect().height) : MIN_EDITOR_HEIGHT;
}

function startResize(e: PointerEvent) {
  e.preventDefault();
  resizingEditor.value = true;
  resizeStartY = e.clientY;
  resizeStartH = editorHeight.value ?? currentEditorHeight();
  window.addEventListener("pointermove", onResizeMove);
  window.addEventListener("pointerup", endResize);
  document.body.classList.add("resizing-editor");
}

function onResizeMove(e: PointerEvent) {
  const h = resizeStartH + (resizeStartY - e.clientY);
  editorHeight.value = Math.min(
    maxEditorHeight(),
    Math.max(MIN_EDITOR_HEIGHT, Math.round(h)),
  );
}

function endResize() {
  resizingEditor.value = false;
  window.removeEventListener("pointermove", onResizeMove);
  window.removeEventListener("pointerup", endResize);
  document.body.classList.remove("resizing-editor");
}

function clampEditorHeightOnResize() {
  if (editorHeight.value != null) {
    editorHeight.value = Math.min(editorHeight.value, maxEditorHeight());
  }
}

// 用户输入历史（仅内存），供向上/向下键选择，行为类似 Linux shell
const sentHistory: string[] = [];
let historyIndex = -1;

const editor = useEditor({
  extensions: [
    StarterKit,
    Placeholder.configure({
      placeholder: "输入消息，@ 引用文件或插件 / $ 调用技能…",
    }),
    Reference,
  ],
  editorProps: {
    attributes: {
      class: "rich-input",
      role: "textbox",
      "aria-multiline": "true",
    },
    handleKeyDown: (_view, event) => handleKeydown(event),
    handlePaste: (_view, event) => handlePasteDom(event),
  },
  onCreate: () => {
    syncAfterChange();
    exposeEditor();
    exposeAddAttachment();
  },
  onUpdate: () => {
    syncAfterChange();
  },
  onSelectionUpdate: () => {
    updateMentionFromCaret();
  },
});

function exposeEditor() {
  try {
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ =
      editor.value;
  } catch {
    // 非浏览器环境忽略
  }
}

/** 会话资源面板“添加为会话附件”的全局入口：追加到附件行并同步 store */
function exposeAddAttachment() {
  try {
    (window as unknown as Record<string, unknown>).__CODEX_UI_ADD_ATTACHMENT__ =
      (a: UserInput) => {
        rowAttachments.value.push(a);
        syncAttachments();
        void nextTick(() => editor.value?.commands.focus());
      };
  } catch {
    // 非浏览器环境忽略
  }
}

function currentRuns(): EditorRun[] {
  const ed = editor.value;
  if (!ed) return [];
  return docToRuns(ed.getJSON());
}

function plainTextBeforeCaret(): string {
  const ed = editor.value;
  if (!ed) return "";
  const pos = ed.state.selection.from;
  return ed.state.doc.textBetween(0, pos, "\n", () => "");
}

function updateMentionFromCaret() {
  const textBefore = plainTextBeforeCaret();
  const m = matchMentionToken(textBefore);
    mention.value = m;
    if (!m) {
      fileResults.value = [];
      searchingFiles.value = false;
      debouncedFileSearch.cancel();
      searchSeq++;
      return;
    }
  if (m.kind === "@") scheduleFileSearch(m.token);
}

function syncAfterChange() {
  updateMentionFromCaret();
  const runs = currentRuns();
  hasText.value = runsToText(runs).trim().length > 0;
  syncAttachments();
}

function syncAttachments() {
  const runs = currentRuns();
  const refs = runs
    .filter((r) => r.kind === "ref")
    .map((r) => (r.kind === "ref" ? refsById.value.get(r.refId) : undefined))
    .filter((a): a is UserInput => !!a);
  store.attachments = [...refs, ...rowAttachments.value];
}

function scheduleFileSearch(token: string) {
  if (!token) {
    fileResults.value = [];
    searchingFiles.value = false;
    debouncedFileSearch.cancel();
    searchSeq++;
    return;
  }
  debouncedFileSearch.run(token);
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
      setToast(toastError(e));
    }
  } finally {
    if (seq === searchSeq) searchingFiles.value = false;
  }
}

function mentionRoot(): string {
  return resolveCwd();
}

function refNameOf(a: UserInput): string {
  if (a.type === "mention" || a.type === "skill") return a.name;
  return "";
}

/** 菜单选中引用：插件/技能删除触发词后插入内联 chip；文件/图片删除触发词后进附件区 */
function onSelectAttachment(a: UserInput) {
  const ed = editor.value;
  if (!ed) return;
  const m = mention.value;
  const caretPos = ed.state.selection.from;
  const from = m
    ? tokenStartPos(ed.state.doc, caretPos, m.token.length)
    : caretPos;
  if (a.type === "mention" || a.type === "localImage") {
    ed.chain().focus().deleteRange({ from, to: caretPos }).run();
    mention.value = null;
    debouncedFileSearch.cancel();
    fileResults.value = [];
    searchingFiles.value = false;
    rowAttachments.value.push(a);
    syncAttachments();
    void nextTick(() => ed.commands.focus());
    return;
  }
  const id = `ref-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  refsById.value.set(id, a);
  ed.chain()
    .focus()
    .deleteRange({ from, to: caretPos })
    .insertContent([
      {
        type: "reference",
        attrs: { refId: id, kind: refKindOfAttachment(a), label: refNameOf(a) },
      },
      { type: "text", text: " " },
    ])
    .run();
  mention.value = null;
  debouncedFileSearch.cancel();
  fileResults.value = [];
  searchingFiles.value = false;
  syncAttachments();
  void nextTick(() => ed.commands.focus());
}

/** 删除当前触发词（用于打开本地文件/文件夹选择器前） */
function removeMentionTokenInEditor() {
  const ed = editor.value;
  const m = mention.value;
  if (!ed || !m) return;
  const caretPos = ed.state.selection.from;
  const from = tokenStartPos(ed.state.doc, caretPos, m.token.length);
  ed.chain().focus().deleteRange({ from, to: caretPos }).run();
  mention.value = null;
  debouncedFileSearch.cancel();
  fileResults.value = [];
  searchingFiles.value = false;
}

function onPickFiles() {
  removeMentionTokenInEditor();
  void (async () => {
    try {
      const files = await invoke<string[]>("pick_files", {
        multiple: true,
        initialDir: mentionRoot(),
      });
      for (const f of files) {
        const a = toUserAttachment(baseName(f), f);
        rowAttachments.value.push(a);
      }
      syncAttachments();
    } catch (e) {
      setToast(toastError(e));
    } finally {
      void nextTick(() => editor.value?.commands.focus());
    }
  })();
}

function onPickDir() {
  removeMentionTokenInEditor();
  void (async () => {
    try {
      const dir = await invoke<string | null>("pick_directory", {
        initialDir: mentionRoot(),
      });
      if (dir) {
        const a = toUserAttachment(baseName(dir), dir);
        rowAttachments.value.push(a);
      }
      syncAttachments();
    } catch (e) {
      setToast(toastError(e));
    } finally {
      void nextTick(() => editor.value?.commands.focus());
    }
  })();
}

function closeMenus() {
  store.permOpen = false;
  store.taskOpen = false;
  store.modelOpen = false;
  store.goalOpen = false;
  mention.value = null;
}

type ComposerMenu = "perm" | "task" | "model";

/** 切换三个按钮菜单：打开一个时关闭另外两个，再点一次当前按钮则关闭 */
function toggleMenu(which: ComposerMenu) {
  const willOpen = !store[`${which}Open`];
  store.permOpen = false;
  store.taskOpen = false;
  store.modelOpen = false;
  store.goalOpen = false;
  store[`${which}Open`] = willOpen;
}

function onKeydownGlobal(e: KeyboardEvent) {
  if (e.key === "Escape") closeMenus();
}

function onWindowMousedown(e: MouseEvent) {
  // 三个按钮弹出层：点击外部任意区域自动关闭。用 mousedown 而非 click，
  // 与 GitView 分支弹层一致——WebView2 原生菜单「粘贴」只合成 click、
  // 不合成 mousedown，避免粘贴等操作误关弹层。
  // 用 Element 而非 HTMLElement：点击 svg/path 等 SVG 目标也应正确判断。
  if (!(e.target instanceof Element)) {
    store.permOpen = false;
    store.taskOpen = false;
    store.modelOpen = false;
    store.goalOpen = false;
    return;
  }
  // 弹出层内部与三个触发按钮不自动关闭（按钮自身的 click 负责切换）
  if (
    e.target.closest(".popup-menu") ||
    e.target.closest(".perm-chip, .task-chip, .model-chip, .goal-chip")
  ) {
    return;
  }
  store.permOpen = false;
  store.taskOpen = false;
  store.modelOpen = false;
  store.goalOpen = false;
}

onMounted(() => {
  window.addEventListener("keydown", onKeydownGlobal);
  window.addEventListener("mousedown", onWindowMousedown);
  window.addEventListener("resize", clampEditorHeightOnResize);
  exposeEditor();
  void setupDragDrop();
  void nextTick(() => editor.value?.commands.focus());
});
watch(editor, () => {
  exposeEditor();
  exposeAddAttachment();
});
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydownGlobal);
  window.removeEventListener("mousedown", onWindowMousedown);
  window.removeEventListener("resize", clampEditorHeightOnResize);
  dropUnlisten?.();
  endResize();
  debouncedFileSearch.cancel();
  try {
    (window as unknown as Record<string, unknown>).__CODEX_UI_EDITOR__ =
      undefined;
  } catch {
    // ignore
  }
});

// 新建对话后输入框重新获得焦点（组件未卸载的情况，如聊天页直接点“新建对话”）
watch(
  () => store.currentThreadId,
  (v) => {
    if (!v) void nextTick(() => editor.value?.commands.focus());
  },
);

function handleKeydown(e: KeyboardEvent): boolean {
  // 输入法组合中（如中文拼音选字）的按键不触发提交/历史选择
  if (e.isComposing || e.keyCode === 229) return false;
  // @ / $ 菜单打开时：Enter 选中高亮项，↑↓ 移动高亮
  if (
    mention.value &&
    (e.key === "ArrowUp" ||
      e.key === "ArrowDown" ||
      (e.key === "Enter" && !e.ctrlKey))
  ) {
    e.preventDefault();
    if (e.key === "Enter") {
      mentionMenu.value?.selectHighlighted();
    } else {
      mentionMenu.value?.move(e.key === "ArrowUp" ? -1 : 1);
    }
    return true;
  }
  if (e.key === "Enter") {
    // Enter 快捷发送：开启时 Enter 发送 / Ctrl+Enter 换行 / Shift+Enter 无操作；
    // 关闭时 Enter 换行 / Ctrl+Enter 发送
    if (store.settings.enter_to_send) {
      if (e.ctrlKey && !e.shiftKey) {
        e.preventDefault();
        editor.value?.commands.setHardBreak();
        return true;
      }
      if (e.shiftKey) {
        e.preventDefault();
        return true;
      }
      e.preventDefault();
      submit(false);
      return true;
    }
    if (e.ctrlKey) {
      e.preventDefault();
      submit(true);
      return true;
    }
    return false;
  }
  if (e.key === "ArrowUp" || e.key === "ArrowDown") {
    // 仅当光标位于文档开头时触发历史选择，否则保留默认的光标移动
    const sel = editor.value?.state.selection;
    if (!sel || !sel.empty || sel.from > 1) return false;
    e.preventDefault();
    if (e.key === "ArrowUp") {
      if (!sentHistory.length) return true;
      if (historyIndex === -1) historyIndex = sentHistory.length - 1;
      else if (historyIndex > 0) historyIndex--;
      setEditorPlainText(sentHistory[historyIndex]);
    } else {
      if (historyIndex === -1) return true;
      historyIndex++;
      if (historyIndex >= sentHistory.length) {
        historyIndex = -1;
        setEditorPlainText("");
      } else {
        setEditorPlainText(sentHistory[historyIndex]);
      }
    }
    return true;
  }
  return false;
}

/** 从剪贴板 MIME/文件名推断图片扩展名（白名单与后端 save_pasted_image 一致） */
function imageExtFromType(type: string, fallbackName: string): string {
  const norm = (ext: string) => (ext === "jpeg" ? "jpg" : ext);
  const m = /^image\/(png|jpe?g|gif|webp|bmp)$/i.exec(type);
  if (m) return norm(m[1].toLowerCase());
  const fn = /\.(png|jpe?g|gif|webp|bmp)$/i.exec(fallbackName);
  if (fn) return norm(fn[1].toLowerCase());
  return "png";
}

/**
 * 图片/文件 → 附件区（粘贴与拖放共用核心逻辑）：
 * - 图片项：优先用原始路径，读不到（截图/网页位图）则落盘；
 * - 非图片文件项：仅支持原始路径，读不到提示暂不支持。
 */
async function addFilesWithPaths(
  files: File[],
  originalPaths: string[],
  source: "粘贴" | "拖放",
) {
  const originalByBase = new Map<string, string>();
  for (const p of originalPaths) {
    const b = baseName(p).toLowerCase();
    if (b && !originalByBase.has(b)) originalByBase.set(b, p);
  }

  let added = 0;
  for (const f of files) {
    const name = f.name || "pasted";
    const orig = originalByBase.get(name.toLowerCase());
    if (f.type.startsWith("image/")) {
      if (orig) {
        rowAttachments.value.push(toUserAttachment(name, orig));
        added++;
        continue;
      }
      if (f.size > MAX_PASTED_IMAGE_BYTES) {
        setToast(`${source}的图片过大（>20MB），已跳过`);
        continue;
      }
      try {
        const bytes = new Uint8Array(await f.arrayBuffer());
        const saved = await invoke<string>("save_pasted_image", {
          bytes: Array.from(bytes),
          name: `pasted.${imageExtFromType(f.type, name)}`,
        });
        rowAttachments.value.push({ type: "localImage", path: saved });
        added++;
      } catch (e) {
        setToast(toastError(e));
      }
    } else if (orig) {
      rowAttachments.value.push(toUserAttachment(name, orig));
      added++;
    } else {
      setToast(`暂不支持该${source}（无法获取原始路径）: ${name}`);
    }
  }
  if (added) {
    syncAttachments();
    await nextTick();
  }
}

/** 粘贴图片/文件 → 附件区（原始路径来自剪贴板 CF_HDROP） */
async function handlePastedFiles(files: File[]) {
  let originalPaths: string[] = [];
  try {
    originalPaths = await invoke<string[]>("clipboard_file_paths");
  } catch {
    originalPaths = [];
  }
  await addFilesWithPaths(files, originalPaths, "粘贴");
}

/** ProseMirror paste 入口：有文件/图片项则消费事件，否则走默认（文本粘贴） */
function handlePasteDom(e: ClipboardEvent): boolean {
  const fileItems = Array.from(e.clipboardData?.items ?? []).filter(
    (it) => it.kind === "file",
  );
  if (!fileItems.length) return false;
  // DataTransferItem 只在 paste 事件同步阶段有效：先取出 File，再异步处理
  const files = fileItems
    .map((it) => it.getAsFile())
    .filter((f): f is File => !!f);
  // 取不到 File（如已失效的 DataTransferItem）：放行默认粘贴，避免吞掉文本
  if (!files.length) return false;
  e.preventDefault();
  void handlePastedFiles(files);
  return true;
}

const dragging = ref(false);
let dropUnlisten: UnlistenFn | undefined;

/** Tauri 拖放事件：drop 时直接拿到绝对路径数组（WebView2 下 HTML5 dataTransfer.files 为空） */
async function setupDragDrop() {
  try {
    dropUnlisten = await getCurrentWebview().onDragDropEvent((event) => {
      const p = event.payload;
      if (p.type === "enter" || p.type === "over") {
        dragging.value = true;
      } else if (p.type === "leave") {
        dragging.value = false;
      } else if (p.type === "drop") {
        dragging.value = false;
        const paths = p.paths ?? [];
        if (paths.length) void addDroppedPaths(paths);
      }
    });
  } catch {
    // 非 Tauri 环境（浏览器/单测）忽略，走 HTML5 drop 兜底
  }
}

/** 拖放路径 → 附件区：图片按扩展名 → localImage，其它 → mention */
async function addDroppedPaths(paths: string[]) {
  let added = 0;
  for (const path of paths) {
    rowAttachments.value.push(toUserAttachment(baseName(path) || "dropped", path));
    added++;
  }
  if (added) {
    syncAttachments();
    await nextTick();
  }
}

function onDragOver() {
  dragging.value = true;
}

function onDragLeave(e: DragEvent) {
  const current = e.currentTarget as HTMLElement | null;
  if (!current || !current.contains(e.relatedTarget as Node | null)) {
    dragging.value = false;
  }
}

/** 拖放图片/文件 → 附件区（原始路径来自拖放 File.path），纯文本拖放放行 */
function onDrop(e: DragEvent) {
  dragging.value = false;
  const files = Array.from(e.dataTransfer?.files ?? []);
  if (!files.length) return;
  e.preventDefault();
  const paths = files
    .map((f) => (f as File & { path?: string }).path)
    .filter((p): p is string => !!p);
  void addFilesWithPaths(files, paths, "拖放");
}

function setEditorPlainText(text: string) {
  editor.value?.commands.setContent(text);
  const ed = editor.value;
  if (ed) {
    const end = ed.state.doc.content.size;
    ed.chain().focus().setTextSelection(end).run();
  }
}

function submit(flip = false) {
  closeMenus(); // 发送后关闭可能开着的菜单，避免回合中还能切换模式
  const runs = currentRuns();
  const wireInline = runsToWireText(runs, refsById.value);
  const plainText = runsToText(runs);
  const refs = runs
    .filter((r) => r.kind === "ref")
    .map((r) => (r.kind === "ref" ? refsById.value.get(r.refId) : undefined))
    .filter((a): a is UserInput => !!a);
  const rowItems = rowAttachments.value;
  const files = rowItems.filter((a) => a.type === "mention");
  const fileSection = fileMentionSection(files);
  const marker = files.length ? `\n${MY_REQUEST_MARKER}\n` : "";
  const wireText = `${fileSection}${marker}${wireInline}`;
  // 先清空编辑器（onUpdate 会同步 store.attachments 为空），再写入本次附件
  editor.value?.commands.setContent("");
  refsById.value = new Map();
  rowAttachments.value = [];
  store.attachments = [...refs, ...rowItems];
  if (plainText.trim()) {
    sentHistory.push(plainText);
    if (sentHistory.length > 100) sentHistory.shift();
  }
  historyIndex = -1;
  void sendPrompt(wireText, flip);
}

function removeRowAttachment(i: number) {
  rowAttachments.value.splice(i, 1);
  syncAttachments();
}

function imageSrc(path: string): string {
  try {
    return convertFileSrc(path);
  } catch {
    return path;
  }
}

function rowAttPath(a: UserInput): string {
  return a.type === "mention" || a.type === "localImage" ? a.path : "";
}

function rowAttName(a: UserInput): string {
  return a.type === "mention" || a.type === "skill" ? a.name : "";
}

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
  if (compacting.value) return "正在压缩上下文…";
  const u = ctxUsage.value;
  if (!u) return "";
  return `上下文已用 ${formatTokens(u.used)}，共 ${formatTokens(u.window)}，双击进行压缩`;
});

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return String(n);
}

/** 发起 thread/compact/start：回合进行中也可压缩，由服务端处理 */
async function compactNow() {
  if (!store.currentThreadId || compacting.value) return;
  compacting.value = true;
  try {
    await invoke("codex_rpc", {
      method: "thread/compact/start",
      params: { threadId: store.currentThreadId },
    });
    setToast("已开始压缩上下文");
  } catch (e) {
    setToast(toastError(e));
  } finally {
    compacting.value = false;
  }
}

function modelChipLabel(): string {
  const name = modelDisplayName(store.model);
  const effort = effectiveEffort();
  return effort ? `${name} (${effort})` : name;
}

function taskModeLabel(): string {
  if (store.taskMode === "plan") return "计划模式";
  return "执行模式";
}

function goalStatusLabel(): string {
  switch (store.goalStatus) {
    case "complete":
      return "✓ 已完成";
    case "budgetLimited":
      return "预算耗尽";
    case "usageLimited":
      return "用量受限";
    case "blocked":
      return "已阻塞";
    case "paused":
      return "已暂停";
    default:
      return "";
  }
}

/** 目标旗子提示：未设置/待应用（会话前预填）/已挂载三种状态 */
function goalTooltip(): string {
  if (!store.goalText) return "设置目标";
  if (!store.currentThreadId) {
    return `目标：${store.goalText}（待应用：创建会话后生效），点击修改`;
  }
  const status =
    store.goalStatus === "active" || !store.goalStatus
      ? "进行中"
      : goalStatusLabel();
  return `目标：${store.goalText}（${status}），点击修改`;
}

/** 目标旗子：始终可点，打开设置弹层（有目标时回填）；再点一次关闭 */
function onGoalIconClick() {
  const willOpen = !store.goalOpen;
  closeMenus();
  store.goalOpen = willOpen;
}

/** × 按钮：直接取消目标（会话前仅清空待填目标），不弹确认 */
function onCancelGoalClick() {
  void clearGoal();
}

</script>

<template>
  <div
    class="composer"
    :class="{ dragover: dragging }"
    @dragover.prevent="onDragOver()"
    @dragleave="onDragLeave($event)"
    @drop="onDrop($event)"
  >
    <div
      class="composer-input-row"
      :style="editorHeight ? { '--editor-h': `${editorHeight}px` } : undefined"
    >
      <div
        class="editor-resize-handle"
        :class="{ active: resizingEditor }"
        aria-label="调整输入框高度"
        v-tooltip="'拖动调整输入框高度'"
        @pointerdown="startResize"
      >
        <span class="editor-resize-grip"></span>
      </div>
      <div class="menu-anchor input-anchor">
        <EditorContent :editor="editor" class="rich-editor" />
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
          <button
            class="perm-chip"
            v-tooltip="'权限模式'"
            :disabled="store.turnActive"
            @click="toggleMenu('perm')"
          >
            <svg class="chip-icon" viewBox="0 0 24 24">
              <path :d="permissionMode(store.permissionMode).icon" />
            </svg>
            {{ permissionChip() }}
            <svg class="chevron" viewBox="0 0 16 16">
              <path :d="ICON_CHEVRON_DOWN" />
            </svg>
          </button>
          <PermissionMenu v-if="store.permOpen" @close="store.permOpen = false" />
        </div>
        <div class="menu-anchor">
          <button
            class="task-chip"
            v-tooltip="'任务模式'"
            :disabled="store.turnActive"
            @click="toggleMenu('task')"
          >
            <svg class="chip-icon" viewBox="0 0 24 24">
              <path :d="taskMode(store.taskMode).icon" />
            </svg>
            {{ taskModeLabel() }}
            <svg viewBox="0 0 16 16">
            <path :d="ICON_CHEVRON_DOWN" />
            </svg>
          </button>
          <TaskModeMenu v-if="store.taskOpen" @close="store.taskOpen = false" />
        </div>
        <div class="menu-anchor">
          <div class="goal-chip">
            <button
              class="goal-icon-btn"
              :class="{
                'has-goal': !!store.goalText,
                'status-complete': store.goalStatus === 'complete',
                'status-budget-limited': store.goalStatus === 'budgetLimited',
                'status-usage-limited': store.goalStatus === 'usageLimited',
                'status-blocked': store.goalStatus === 'blocked',
                'status-paused': store.goalStatus === 'paused',
              }"
              :aria-label="goalTooltip()"
              v-tooltip="goalTooltip()"
              @click="onGoalIconClick()"
            >
              <svg viewBox="0 0 24 24">
                <path :d="ICON_GOAL" />
              </svg>
            </button>
            <button
              v-if="store.goalText"
              class="goal-clear-btn"
              aria-label="取消目标"
              v-tooltip="'取消目标'"
              @click="onCancelGoalClick()"
            >
              <svg viewBox="0 0 24 24">
                <path
                  d="M6.4 5 5 6.4 10.6 12 5 17.6 6.4 19 12 13.4 17.6 19 19 17.6 13.4 12 19 6.4 17.6 5 12 10.6z"
                />
              </svg>
            </button>
          </div>
          <GoalMenu v-if="store.goalOpen" @close="store.goalOpen = false" />
        </div>
      </div>
      <div class="composer-right">
        <button
          v-if="ctxUsage"
          class="ctx-window"
          aria-label="压缩上下文"
          :disabled="!store.currentThreadId || compacting"
          v-tooltip="ctxTooltip"
          @dblclick="compactNow()"
        >
          {{ ctxUsage.pct }}%
        </button>
        <div class="menu-anchor">
          <button
            class="model-chip"
            v-tooltip="'模型'"
            @click="toggleMenu('model')"
          >
            <svg class="model-chip-icon" viewBox="0 0 24 24">
              <rect x="5" y="5" width="14" height="14" rx="2" />
              <rect x="9.5" y="9.5" width="5" height="5" rx="1" />
              <path d="M9 2v3M15 2v3M9 19v3M15 19v3M2 9h3M2 15h3M19 9h3M19 15h3" />
            </svg>
            {{ modelChipLabel() }}
            <svg viewBox="0 0 16 16">
              <path :d="ICON_CHEVRON_DOWN" />
            </svg>
          </button>
          <ModelMenu v-if="store.modelOpen" @close="store.modelOpen = false" />
        </div>
        <button
          v-if="store.turnActive"
          class="send-btn stop"
          v-tooltip="'停止生成'"
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
          v-tooltip="'发送'"
          :class="{ lit: !!(hasText || store.attachments.length) }"
          :disabled="!hasText && store.attachments.length === 0"
          @click="submit()"
        >
          <svg viewBox="0 0 24 24">
            <path d="M12 19V5M5 12l7-7 7 7" />
          </svg>
          发送
        </button>
      </div>
    </div>
    <div v-if="rowAttachments.length" class="attachment-row">
      <span
        v-for="(a, i) in rowAttachments"
        :key="i"
        class="attachment-chip"
        v-tooltip="rowAttPath(a)"
      >
        <img
          v-if="a.type === 'localImage'"
          class="attachment-thumb"
          :src="imageSrc(a.path)"
          alt=""
        />
        <template v-if="a.type === 'localImage'">
          {{ a.path.split(/[\\/]/).pop() ?? a.path }}
        </template>
        <template v-else>@{{ rowAttName(a) }}</template>
        <button
          aria-label="移除附件"
          v-tooltip="'移除附件'"
          @click="removeRowAttachment(i)"
        >
          ×
        </button>
      </span>
    </div>

    <Teleport to="body">
      <div
        v-if="
          store.permOpen ||
          store.taskOpen ||
          store.modelOpen ||
          store.goalOpen ||
          mention
        "
        class="menu-backdrop"
        @click="closeMenus()"
      ></div>
    </Teleport>
  </div>
</template>
