<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Compartment, type Text } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import {
  buildEditorExtensions,
  createEditorState,
  languageForPath,
} from "../lib/editorSetup";
import {
  buildSaveContent,
  detectEol,
  normalizeForEditor,
  stripBom,
  type EditorEol,
} from "../lib/editorFile";
import { languageFromPath } from "../lib/highlight";
import { formatFileSize } from "../lib/sessionFs";
import { setWindowTitleFromPath } from "../lib/windowTitle";

interface TextEditorParams {
  root: string;
  path: string;
}

interface TextFileContent {
  content: string;
  validUtf8: boolean;
  byteSize: number;
}

interface EditorConfirm {
  message: string;
  saveLabel: string;
  discardLabel: string;
  onSave: () => void;
  onDiscard: () => void;
}

const loading = ref(true);
const error = ref("");
const params = ref<TextEditorParams | null>(null);
const readOnly = ref(false);
const dirty = ref(false);
const wrap = ref(false);
const saving = ref(false);
const cursor = ref({ line: 1, col: 1 });
const eol = ref<EditorEol>("\n");
const hadBom = ref(false);
const byteSize = ref<number | null>(null);
const status = ref("");
const confirm = ref<EditorConfirm | null>(null);
const editorHost = ref<HTMLDivElement | null>(null);

const lang = computed(() =>
  params.value ? languageFromPath(params.value.path) : null,
);
const langLabel = computed(() => lang.value ?? "text");
const byteSizeLabel = computed(() =>
  byteSize.value == null ? "" : formatFileSize(byteSize.value),
);

let view: EditorView | null = null;
let savedText: Text | null = null;
let wrapCompartment: Compartment | null = null;
let unlistenOpen: UnlistenFn | null = null;
let unlistenClose: UnlistenFn | null = null;

function nowTime(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

/** 销毁旧编辑器并按当前文件重建（文件切换时重置历史与脏状态） */
function mountEditor(doc: string, ro: boolean) {
  const host = editorHost.value;
  if (!host) return;
  view?.destroy();
  wrapCompartment = new Compartment();
  const ext = buildEditorExtensions({
    language: params.value ? languageForPath(params.value.path) : null,
    readOnly: ro,
    wrap: wrap.value,
    wrapCompartment,
    savedText: () => savedText,
    onDirtyChange: (d) => {
      dirty.value = d;
    },
    onCursorChange: (line, col) => {
      cursor.value = { line, col };
    },
    onSave: () => {
      void save();
    },
  });
  const state = createEditorState(doc, ext);
  savedText = state.doc;
  view = new EditorView({ state, parent: host });
  // 编辑区内放行原生右键菜单（剪切/复制/粘贴）；窗口其余区域仍屏蔽
  view.dom.addEventListener("contextmenu", (e) => e.stopPropagation());
  // 测试挂点：组件测试通过 host.__cmView 驱动编辑
  (host as HTMLDivElement & { __cmView?: EditorView }).__cmView = view;
  view.focus();
}

async function loadFile(root: string, path: string) {
  loading.value = true;
  error.value = "";
  status.value = "";
  dirty.value = false;
  // 窗口标题栏显示文件名（不含路径），切换文件时同步
  await setWindowTitleFromPath(path);
  try {
    const info = await invoke<TextFileContent>("session_fs_read", {
      root,
      path,
    });
    params.value = { root, path };
    byteSize.value = info.byteSize;
    const ro = !info.validUtf8;
    readOnly.value = ro;
    const { text, hadBom: bom } = stripBom(info.content);
    const lineEol = detectEol(text);
    eol.value = lineEol;
    hadBom.value = bom;
    const doc = normalizeForEditor(text, lineEol);
    // 先渲染编辑器容器（v-else 分支），再挂载 CodeMirror
    loading.value = false;
    await nextTick();
    await mountEditor(doc, ro);
    if (ro) {
      status.value = "文件不是 UTF-8 编码，已以只读方式打开";
    }
  } catch (e) {
    error.value = String(e);
    readOnly.value = false;
    byteSize.value = null;
  } finally {
    loading.value = false;
  }
}

async function save(): Promise<boolean> {
  if (!view || !params.value || readOnly.value || !dirty.value || saving.value) {
    return false;
  }
  saving.value = true;
  try {
    const content = buildSaveContent(
      view.state.doc.toString(),
      eol.value,
      hadBom.value,
    );
    await invoke("session_fs_write", {
      root: params.value.root,
      path: params.value.path,
      content,
    });
    savedText = view.state.doc;
    dirty.value = false;
    status.value = `已保存 ${nowTime()}`;
    return true;
  } catch (e) {
    status.value = `保存失败：${String(e)}`;
    return false;
  } finally {
    saving.value = false;
  }
}

function toggleWrap() {
  wrap.value = !wrap.value;
  if (view && wrapCompartment) {
    view.dispatch({
      effects: wrapCompartment.reconfigure(
        wrap.value ? EditorView.lineWrapping : [],
      ),
    });
  }
}

function requestOpenFile(root: string, path: string) {
  if (dirty.value) {
    confirm.value = {
      message: `「${path}」有未保存的更改，切换文件将丢失这些更改。`,
      saveLabel: "保存并切换",
      discardLabel: "放弃并切换",
      onSave: () => {
        void (async () => {
          confirm.value = null;
          const ok = await save();
          if (ok) await loadFile(root, path);
        })();
      },
      onDiscard: () => {
        confirm.value = null;
        void loadFile(root, path);
      },
    };
    return;
  }
  void loadFile(root, path);
}

async function setupCloseGuard() {
  try {
    const win = getCurrentWindow();
    unlistenClose = await win.onCloseRequested(async (event) => {
      if (!dirty.value || confirm.value) return;
      event.preventDefault();
      confirm.value = {
        message: `「${params.value?.path ?? ""}」有未保存的更改，关闭窗口将丢失这些更改。`,
        saveLabel: "保存并关闭",
        discardLabel: "放弃更改",
        onSave: () => {
          void (async () => {
            confirm.value = null;
            const ok = await save();
            if (ok) await win.destroy();
          })();
        },
        onDiscard: () => {
          confirm.value = null;
          void win.destroy();
        },
      };
    });
  } catch {
    // 非 Tauri 环境（单测/浏览器）忽略
  }
}

function onBeforeUnload(e: BeforeUnloadEvent) {
  if (dirty.value) {
    e.preventDefault();
    e.returnValue = "";
  }
}

function onContextMenu(e: Event) {
  e.preventDefault();
}

onMounted(async () => {
  window.addEventListener("contextmenu", onContextMenu);
  window.addEventListener("beforeunload", onBeforeUnload);
  await setupCloseGuard();
  try {
    unlistenOpen = await listen<TextEditorParams>("text-editor/open", (ev) => {
      requestOpenFile(ev.payload.root, ev.payload.path);
    });
  } catch {
    // 非 Tauri 环境（单测/浏览器）忽略
  }
  const p = await invoke<TextEditorParams | null>("take_text_editor_params");
  if (p) {
    await loadFile(p.root, p.path);
  } else {
    error.value = "未找到文本预览参数";
    loading.value = false;
  }
});

onBeforeUnmount(() => {
  window.removeEventListener("contextmenu", onContextMenu);
  window.removeEventListener("beforeunload", onBeforeUnload);
  unlistenOpen?.();
  unlistenClose?.();
  view?.destroy();
  view = null;
});
</script>

<template>
  <div class="text-editor-window">
    <div class="text-editor-head">
      <span class="text-editor-title">
        <span class="text-editor-path">{{ params?.path ?? "" }}</span>
        <span v-if="params" class="text-editor-lang">{{ langLabel }}</span>
        <span v-if="dirty" class="text-editor-dirty">未保存</span>
        <span v-if="readOnly" class="text-editor-ro">只读</span>
      </span>
      <span class="text-editor-actions">
        <button
          class="text-editor-icon-btn"
          :class="{ active: wrap }"
          aria-label="换行"
          v-tooltip="'换行'"
          @click="toggleWrap"
        >
          <svg viewBox="0 0 24 24">
            <path
              d="M4 19h6v-2H4v2zM20 5H4v2h16V5zm-3 6H4v2h13.25c1.1 0 2 .9 2 2s-.9 2-2 2H15v-2l-3 3 3 3v-2h2c2.21 0 4-1.79 4-4s-1.79-4-4-4z"
            />
          </svg>
        </button>
        <button
          class="text-editor-icon-btn primary"
          :disabled="!dirty || readOnly || saving"
          aria-label="保存"
          v-tooltip="'保存'"
          @click="save"
        >
          <svg viewBox="0 0 24 24">
            <path
              d="M6 3h11l4 4v13a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zm1 2v4h8V5H7zm0 9v5h10v-5H7z"
            />
          </svg>
        </button>
      </span>
    </div>
    <div v-if="loading" class="text-editor-note">正在加载文件内容…</div>
    <div v-else-if="error" class="text-editor-note">
      无法编辑该文件（{{ error }}）
    </div>
    <div v-else class="text-editor-body">
      <div v-if="readOnly" class="text-editor-banner">
        该文件不是 UTF-8 编码，已以只读方式打开（保存功能已禁用）。
      </div>
      <div ref="editorHost" class="text-editor-host"></div>
    </div>
    <div class="text-editor-status">
      <span>行 {{ cursor.line }}，列 {{ cursor.col }}</span>
      <span>{{ langLabel }}</span>
      <span>UTF-8{{ readOnly ? "（只读）" : "" }}</span>
      <span v-if="byteSizeLabel">{{ byteSizeLabel }}</span>
      <span>{{ eol === "\r\n" ? "CRLF" : "LF" }}</span>
      <span class="text-editor-status-save">{{ status }}</span>
    </div>
    <div v-if="confirm" class="text-editor-overlay">
      <div class="text-editor-confirm">
        <div class="text-editor-confirm-msg">{{ confirm.message }}</div>
        <div class="text-editor-confirm-actions">
          <button
            class="text-editor-btn primary"
            @click="confirm.onSave()"
          >
            {{ confirm.saveLabel }}
          </button>
          <button class="text-editor-btn" @click="confirm.onDiscard()">
            {{ confirm.discardLabel }}
          </button>
          <button class="text-editor-btn" @click="confirm = null">取消</button>
        </div>
      </div>
    </div>
  </div>
</template>
