<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { EditorView } from "@codemirror/view";
import { formatFileSize } from "../lib/sessionFs";
import { languageFromPath } from "../lib/highlight";
import { relPathOf } from "../lib/format";
import { saveFileTab, type FileEditorTab } from "../composables/useEditorTabs";

const props = defineProps<{ tab: FileEditorTab }>();

const editorHost = ref<HTMLDivElement | null>(null);
let view: EditorView | null = null;

const langLabel = computed(
  () => languageFromPath(props.tab.path) ?? "text",
);
const byteSizeLabel = computed(() =>
  props.tab.byteSize == null
    ? ""
    : formatFileSize(props.tab.byteSize),
);

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
  // 编辑区内放行原生右键菜单（剪切/复制/粘贴）；面板其余区域仍走自定义菜单
  view.dom.addEventListener("contextmenu", (e) => e.stopPropagation());
  // 测试挂点：组件测试通过 host.__cmView 驱动编辑
  (host as HTMLDivElement & { __cmView?: EditorView }).__cmView = view;
  view.focus();
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

onMounted(() => {
  ensureView();
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

onBeforeUnmount(() => {
  view?.destroy();
  view = null;
});
</script>

<template>
  <div class="text-editor-window text-editor-embedded">
    <div class="text-editor-head">
      <span class="text-editor-title">
        <span class="text-editor-path">{{ relPathOf(tab.root, tab.path) }}</span>
        <span class="text-editor-lang">{{ langLabel }}</span>
        <span v-if="tab.dirty" class="text-editor-dirty">未保存</span>
        <span v-if="tab.readOnly" class="text-editor-ro">只读</span>
      </span>
      <span class="text-editor-actions">
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
      <div ref="editorHost" class="text-editor-host"></div>
    </div>
    <div class="text-editor-status">
      <span>行 {{ tab.cursor.line }}，列 {{ tab.cursor.col }}</span>
      <span>{{ langLabel }}</span>
      <span>UTF-8{{ tab.readOnly ? "（只读）" : "" }}</span>
      <span v-if="byteSizeLabel">{{ byteSizeLabel }}</span>
      <span>{{ tab.eol === "\r\n" ? "CRLF" : "LF" }}</span>
      <span class="text-editor-status-save">{{ tab.status }}</span>
    </div>
  </div>
</template>
