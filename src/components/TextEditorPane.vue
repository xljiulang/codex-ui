<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { EditorView } from "@codemirror/view";
import { redo, selectAll, undo } from "@codemirror/commands";
import { openSearchPanel } from "@codemirror/search";
import { formatFileSize } from "../lib/sessionFs";
import { languageFromPath } from "../lib/highlight";
import { relPathOf } from "../lib/format";
import { saveFileTab, type FileEditorTab } from "../composables/useEditorTabs";
import { useActionMenu, type CtxItem } from "../composables/useActionMenu";
import { setToast } from "../composables/useCodex";
import { copyText } from "../lib/clipboard";
import { formatDoc, isFormattablePath } from "../lib/codeFormat";
import { exportMarkdownToPdf } from "../lib/pdfExport";
import MarkdownText from "./MarkdownText.vue";
import {
  ICON_COPY,
  ICON_CUT,
  ICON_EDIT,
  ICON_FIND,
  ICON_FORMAT,
  ICON_PASTE,
  ICON_PDF,
  ICON_PREVIEW,
  ICON_REDO,
  ICON_SELECT_ALL,
  ICON_UNDO,
} from "../lib/icons";

const props = defineProps<{ tab: FileEditorTab }>();

const editorHost = ref<HTMLDivElement | null>(null);
let view: EditorView | null = null;

/** Markdown 文件（.md/.markdown）显示「预览/编辑」切换 */
const isMarkdown = computed(
  () => languageFromPath(props.tab.path) === "markdown",
);
/** 预览/编辑态随标签持久化：切走再回来保持切出时状态（存于 FileEditorTab.markdownPreview） */
const previewMode = computed({
  get: () => props.tab.markdownPreview,
  set: (v: boolean) => {
    props.tab.markdownPreview = v;
  },
});

const langLabel = computed(
  () => languageFromPath(props.tab.path) ?? "text",
);
const byteSizeLabel = computed(() =>
  props.tab.byteSize == null
    ? ""
    : formatFileSize(props.tab.byteSize),
);

/**
 * 预览内容：以标签保存的 EditorState 为唯一数据源（onStateChange 在每次
 * 文档/选区更新后同步回写）。不读挂载中的 view——它是非响应式变量，切换
 * 标签后 computed 不会随视图换档失效，会导致预览显示上一标签的内容。
 */
const previewText = computed(() => props.tab.editorState?.doc.toString() ?? "");

const {
  ctxMenu,
  openCtx,
  onWindowClick,
  onWindowScroll,
  onKeydown: onMenuKeydown,
} = useActionMenu({ width: 190, scrollScope: ".text-editor-embedded" });

/** 挂载/切换 CodeMirror 视图：首次创建，之后用标签内保存的 EditorState 直接替换 */
function ensureView() {
  const host = editorHost.value;
  const state = props.tab.editorState;
  if (!host || !state) return;
  // 加载/错误分支切换会让宿主 div 重建：旧视图挂在已脱离 DOM 的旧宿主上，需在新宿主重建
  if (view && view.dom.parentElement !== host) {
    view.destroy();
    view = null;
  }
  if (view) {
    if (view.state !== state) {
      view.setState(state);
      view.focus();
    }
    view.requestMeasure();
    return;
  }
  view = new EditorView({ state, parent: host });
  // 编辑区自定义右键菜单（剪切/复制/粘贴等常用功能），面板其余区域仍走全局菜单
  view.dom.addEventListener("contextmenu", onEditorContextMenu);
  // 测试挂点：组件测试通过 host.__cmView 驱动编辑
  (host as HTMLDivElement & { __cmView?: EditorView }).__cmView = view;
  view.focus();
}

/** 编辑区右键：定位光标到点击处（点击处不在选区时），再弹出自定义菜单 */
function onEditorContextMenu(e: MouseEvent) {
  if (!view) return;
  e.preventDefault();
  e.stopPropagation();
  const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
  if (pos != null) {
    const inside = view.state.selection.ranges.some(
      (r) => r.from <= pos && r.to >= pos,
    );
    if (!inside) {
      view.dispatch({ selection: { anchor: pos } });
    }
  }
  openCtx(e, buildMenuItems());
}

/** 常用功能菜单：剪切/复制仅在存在选区时显示 */
function buildMenuItems(): CtxItem[] {
  const hasSelection = view ? !view.state.selection.main.empty : false;
  const items: CtxItem[] = [
    {
      label: "撤销",
      icon: ICON_UNDO,
      action: () => {
        if (view) {
          undo(view);
          view.focus();
        }
      },
    },
    {
      label: "重做",
      icon: ICON_REDO,
      action: () => {
        if (view) {
          redo(view);
          view.focus();
        }
      },
    },
  ];
  if (hasSelection) {
    items.push(
      { label: "剪切", icon: ICON_CUT, action: () => void cutSelection() },
      { label: "复制", icon: ICON_COPY, action: () => void copySelection() },
    );
  }
  items.push(
    { label: "粘贴", icon: ICON_PASTE, action: () => void pasteAtCursor() },
    {
      label: "全选",
      icon: ICON_SELECT_ALL,
      action: () => {
        if (view) {
          selectAll(view);
          view.focus();
        }
      },
    },
    {
      label: "查找/替换",
      icon: ICON_FIND,
      action: () => {
        if (view) openSearchPanel(view);
      },
    },
  );
  if (isFormattablePath(props.tab.path)) {
    items.push({
      label: "代码格式化",
      icon: ICON_FORMAT,
      action: () => void formatCurrentDoc(),
    });
  }
  if (isMarkdown.value && !props.tab.loading && !props.tab.error) {
    items.push({
      label: "导出 PDF",
      icon: ICON_PDF,
      action: () => void exportMarkdownToPdf(props.tab),
    });
  }
  return items;
}

/** 代码格式化：全文重排为单次可撤销事务；失败或加载中不修改文档 */
async function formatCurrentDoc() {
  const v = view;
  if (!v) return;
  const res = await formatDoc(props.tab.path, v.state.doc.toString());
  if (!res.ok) {
    setToast(res.message);
    return;
  }
  if ("unchanged" in res) return;
  v.dispatch({
    changes: { from: 0, to: v.state.doc.length, insert: res.text },
  });
  v.focus();
}

async function copySelection() {
  const v = view;
  if (!v) return;
  const sel = v.state.selection.main;
  if (sel.empty) return;
  await copyText(v.state.sliceDoc(sel.from, sel.to));
}

async function cutSelection() {
  const v = view;
  if (!v) return;
  const sel = v.state.selection.main;
  if (sel.empty) return;
  const ok = await copyText(v.state.sliceDoc(sel.from, sel.to));
  if (ok) {
    v.dispatch(v.state.replaceSelection(""));
    v.focus();
  }
}

/** 粘贴：WebView2 不支持 execCommand("paste")，经 Clipboard API 读取后插入选区 */
async function pasteAtCursor() {
  const v = view;
  if (!v) return;
  let text = "";
  try {
    text = await navigator.clipboard.readText();
  } catch {
    setToast("无法读取剪贴板");
    return;
  }
  const { from, to } = v.state.selection.main;
  v.dispatch({
    changes: { from, to, insert: text },
    selection: { anchor: from + text.length },
  });
  v.focus();
}

function toggleWrap() {
  const tab = props.tab;
  if (!tab.wrapCompartment || !view) return;
  tab.wrap = !tab.wrap;
  view.dispatch({
    effects: tab.wrapCompartment.reconfigure(
      tab.wrap ? EditorView.lineWrapping : [],
    ),
  });
}

function onWindowKeydown(e: KeyboardEvent) {
  if (onMenuKeydown(e)) return;
  // 预览态下编辑器未聚焦，CodeMirror 快捷键不生效：窗口层兜底 Ctrl+S 保存
  if (
    previewMode.value &&
    (e.ctrlKey || e.metaKey) &&
    e.key.toLowerCase() === "s"
  ) {
    e.preventDefault();
    void saveFileTab(props.tab.id);
  }
}

onMounted(() => {
  ensureView();
  window.addEventListener("keydown", onWindowKeydown);
  window.addEventListener("click", onWindowClick);
  window.addEventListener("scroll", onWindowScroll, true);
});

// 标签切换（组件保持挂载时 prop 变化）或异步加载完成后重建视图；
// 加载分支下宿主尚未渲染，需等 nextTick 拿到新宿主再挂载/切换
watch(
  () => props.tab,
  () => {
    void nextTick(ensureView);
  },
);
watch(
  () => [props.tab.loading, props.tab.editorState] as const,
  () => {
    void nextTick(ensureView);
  },
);
// 回到编辑态时宿主重新可见：补一次测量，避免预览态下隐藏宿主中创建的视图布局滞后
watch(previewMode, (v) => {
  if (!v) void nextTick(() => view?.requestMeasure());
});

onBeforeUnmount(() => {
  view?.destroy();
  view = null;
  window.removeEventListener("keydown", onWindowKeydown);
  window.removeEventListener("click", onWindowClick);
  window.removeEventListener("scroll", onWindowScroll, true);
});
</script>

<template>
  <div class="text-editor-window text-editor-embedded">
    <div class="text-editor-head">
      <span class="text-editor-title">
        <span class="text-editor-path">{{ relPathOf(tab.workspace, tab.path) }}</span>
        <span class="text-editor-lang">{{ langLabel }}</span>
        <span v-if="tab.dirty" class="text-editor-dirty">未保存</span>
        <span v-if="tab.readOnly" class="text-editor-ro">只读</span>
      </span>
      <span class="text-editor-actions">
        <button
          v-if="isMarkdown"
          class="text-editor-icon-btn"
          :class="{ active: previewMode }"
          :aria-label="previewMode ? '编辑' : '预览'"
          v-tooltip="previewMode ? '编辑' : '预览'"
          @click="previewMode = !previewMode"
        >
          <svg viewBox="0 0 24 24">
            <path :d="previewMode ? ICON_EDIT : ICON_PREVIEW" />
          </svg>
        </button>
        <button
          class="text-editor-icon-btn"
          :class="{ active: tab.wrap }"
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
          :disabled="!tab.dirty || tab.readOnly || tab.saving"
          aria-label="保存"
          v-tooltip="'保存'"
          @click="saveFileTab(tab.id)"
        >
          <svg viewBox="0 0 24 24">
            <path
              d="M6 3h11l4 4v13a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zm1 2v4h8V5H7zm0 9v5h10v-5H7z"
            />
          </svg>
        </button>
      </span>
    </div>
    <div v-if="tab.loading" class="text-editor-note">正在加载文件内容…</div>
    <div v-else-if="tab.error" class="text-editor-note">
      无法编辑该文件（{{ tab.error }}）
    </div>
    <div v-else class="text-editor-body">
      <div v-if="tab.readOnly" class="text-editor-banner">
        该文件不是 UTF-8 编码，已以只读方式打开（保存功能已禁用）。
      </div>
      <div v-show="!previewMode" ref="editorHost" class="text-editor-host"></div>
      <div v-if="previewMode" class="text-editor-preview">
        <MarkdownText :text="previewText" />
      </div>
    </div>
    <div class="text-editor-status">
      <span v-if="!previewMode">行 {{ tab.cursor.line }}，列 {{ tab.cursor.col }}</span>
      <span>{{ langLabel }}</span>
      <span>UTF-8{{ tab.readOnly ? "（只读）" : "" }}</span>
      <span v-if="byteSizeLabel">{{ byteSizeLabel }}</span>
      <span>{{ tab.eol === "\r\n" ? "CRLF" : "LF" }}</span>
      <span class="text-editor-status-save">{{ tab.status }}</span>
    </div>
    <div
      v-if="ctxMenu"
      class="ctx-menu"
      :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }"
      @click.stop
    >
      <button
        v-for="it in ctxMenu.items"
        :key="it.label"
        class="ctx-menu-item"
        @click="it.action(); ctxMenu = null"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path :d="it.icon" fill-rule="evenodd" />
        </svg>
        <span>{{ it.label }}</span>
      </button>
    </div>
  </div>
</template>
