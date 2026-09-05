<script setup lang="ts">
import {
  computed,
  markRaw,
  onBeforeUnmount,
  onMounted,
  ref,
  watch,
} from "vue";
import { Editor, EditorContent } from "@tiptap/vue-3";
import { formatFileSize } from "../lib/sessionFs";
import { relPathOf } from "../lib/format";
import { saveDocxTab, type DocxEditorTab } from "../composables/useEditorTabs";
import { docxEditorExtensions } from "../lib/docxEditor";
import {
  ICON_BOLD,
  ICON_CODE,
  ICON_H1,
  ICON_H2,
  ICON_IMAGE,
  ICON_ITALIC,
  ICON_LINK,
  ICON_LIST_OL,
  ICON_LIST_UL,
  ICON_QUOTE,
  ICON_REDO,
  ICON_SAVE,
  ICON_STRIKE,
  ICON_TABLE,
  ICON_UNDERLINE,
  ICON_UNDO,
} from "../lib/icons";

const props = defineProps<{ tab: DocxEditorTab }>();

const imageInput = ref<HTMLInputElement | null>(null);
/** 工具栏激活态随选区/文档变化刷新（TipTap 状态本身非响应式） */
const uiVersion = ref(0);

/** 首次创建 TipTap 实例；切走再回来复用同一实例（编辑内容与撤销历史不丢） */
function ensureEditor() {
  if (props.tab.editor || !props.tab.initialHtml) return;
  const editor = new Editor({
    // 先挂到临时元素，EditorContent 挂载时会把视图迁移到真实宿主
    element: document.createElement("div"),
    extensions: docxEditorExtensions(),
    content: props.tab.initialHtml,
    editorProps: {
      attributes: { class: "docx-editor-prosemirror" },
    },
    onUpdate: () => {
      props.tab.dirty = true;
      uiVersion.value++;
    },
    onSelectionUpdate: () => {
      uiVersion.value++;
    },
  });
  props.tab.editor = markRaw(editor);
  uiVersion.value++;
}

const byteSizeLabel = computed(() =>
  props.tab.byteSize == null ? "" : formatFileSize(props.tab.byteSize),
);

function isActive(type: string, attrs?: Record<string, unknown>): boolean {
  const e = props.tab.editor;
  void uiVersion.value;
  return e ? e.isActive(type, attrs) : false;
}

function run(fn: (e: Editor) => void): void {
  const e = props.tab.editor;
  if (e) {
    fn(e);
    e.commands.focus();
  }
}

function onKeydown(e: KeyboardEvent) {
  if (
    (e.ctrlKey || e.metaKey) &&
    e.key.toLowerCase() === "s" &&
    props.tab.dirty &&
    !props.tab.saving
  ) {
    e.preventDefault();
    void saveDocxTab(props.tab.id);
  }
}

function onImageFile() {
  const input = imageInput.value;
  const file = input?.files?.[0];
  if (!file || !props.tab.editor) return;
  const reader = new FileReader();
  reader.onload = () => {
    if (typeof reader.result === "string") {
      run((e) => e.chain().focus().setImage({ src: reader.result as string }).run());
    }
  };
  reader.readAsDataURL(file);
  input.value = "";
}

function onInsertLink() {
  const e = props.tab.editor;
  if (!e) return;
  const url = window.prompt(
    "输入链接地址（http/https/mailto）",
    "https://",
  );
  if (url === null) return;
  run((ed) =>
    url.trim() === ""
      ? ed.chain().focus().unsetLink().run()
      : ed.chain().focus().toggleLink({ href: url.trim() }).run(),
  );
}

onMounted(() => {
  ensureEditor();
  window.addEventListener("keydown", onKeydown);
});

// 按标签 id 与加载完成态重建/创建编辑器：切标签时 EditorContent 以
// :key="tab.id" 重建宿主，旧编辑器 DOM 随卸载移除，避免多个 ProseMirror
// 视图堆在同一宿主内导致内容错乱；initialHtml 覆盖异步加载完成路径。
watch(
  [() => props.tab.id, () => props.tab.initialHtml],
  () => ensureEditor(),
);

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  // 编辑器实例随标签保留（切换标签不销毁），下次挂载 EditorContent 自动迁移视图
});
</script>

<template>
  <div class="docx-editor-pane docx-editor-embedded">
    <div class="docx-editor-head">
      <span class="docx-editor-title">
        <span class="docx-editor-path">{{ relPathOf(tab.workspace, tab.path) }}</span>
        <span class="docx-editor-lang">Word 文档</span>
        <span v-if="tab.dirty" class="docx-editor-dirty">未保存</span>
      </span>
    </div>
    <div v-if="tab.loading" class="docx-editor-note">正在加载文档内容…</div>
    <div v-else-if="tab.error" class="docx-editor-note">
      无法编辑该文件（{{ tab.error }}）
    </div>
    <div v-else-if="tab.initialHtml" class="docx-editor-body">
      <div class="docx-editor-toolbar">
        <button
          class="docx-editor-tool-btn"
          aria-label="撤销"
          v-tooltip="'撤销'"
          @click="run((e) => e.chain().focus().undo().run())"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_UNDO" /></svg>
        </button>
        <button
          class="docx-editor-tool-btn"
          aria-label="重做"
          v-tooltip="'重做'"
          @click="run((e) => e.chain().focus().redo().run())"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_REDO" /></svg>
        </button>
        <span class="docx-editor-tool-sep" />
        <button
          class="docx-editor-tool-btn"
          :class="{ active: isActive('heading', { level: 1 }) }"
          aria-label="标题 1"
          v-tooltip="'标题 1'"
          @click="run((e) => e.chain().focus().toggleHeading({ level: 1 }).run())"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_H1" /></svg>
        </button>
        <button
          class="docx-editor-tool-btn"
          :class="{ active: isActive('heading', { level: 2 }) }"
          aria-label="标题 2"
          v-tooltip="'标题 2'"
          @click="run((e) => e.chain().focus().toggleHeading({ level: 2 }).run())"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_H2" /></svg>
        </button>
        <span class="docx-editor-tool-sep" />
        <button
          class="docx-editor-tool-btn"
          :class="{ active: isActive('bold') }"
          aria-label="加粗"
          v-tooltip="'加粗'"
          @click="run((e) => e.chain().focus().toggleBold().run())"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_BOLD" /></svg>
        </button>
        <button
          class="docx-editor-tool-btn"
          :class="{ active: isActive('italic') }"
          aria-label="斜体"
          v-tooltip="'斜体'"
          @click="run((e) => e.chain().focus().toggleItalic().run())"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_ITALIC" /></svg>
        </button>
        <button
          class="docx-editor-tool-btn"
          :class="{ active: isActive('underline') }"
          aria-label="下划线"
          v-tooltip="'下划线'"
          @click="run((e) => e.chain().focus().toggleUnderline().run())"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_UNDERLINE" /></svg>
        </button>
        <button
          class="docx-editor-tool-btn"
          :class="{ active: isActive('strike') }"
          aria-label="删除线"
          v-tooltip="'删除线'"
          @click="run((e) => e.chain().focus().toggleStrike().run())"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_STRIKE" /></svg>
        </button>
        <span class="docx-editor-tool-sep" />
        <button
          class="docx-editor-tool-btn"
          :class="{ active: isActive('bulletList') }"
          aria-label="无序列表"
          v-tooltip="'无序列表'"
          @click="run((e) => e.chain().focus().toggleBulletList().run())"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_LIST_UL" /></svg>
        </button>
        <button
          class="docx-editor-tool-btn"
          :class="{ active: isActive('orderedList') }"
          aria-label="有序列表"
          v-tooltip="'有序列表'"
          @click="run((e) => e.chain().focus().toggleOrderedList().run())"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_LIST_OL" /></svg>
        </button>
        <button
          class="docx-editor-tool-btn"
          :class="{ active: isActive('blockquote') }"
          aria-label="引用"
          v-tooltip="'引用'"
          @click="run((e) => e.chain().focus().toggleBlockquote().run())"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_QUOTE" /></svg>
        </button>
        <button
          class="docx-editor-tool-btn"
          :class="{ active: isActive('codeBlock') }"
          aria-label="代码块"
          v-tooltip="'代码块'"
          @click="run((e) => e.chain().focus().toggleCodeBlock().run())"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_CODE" /></svg>
        </button>
        <span class="docx-editor-tool-sep" />
        <button
          class="docx-editor-tool-btn"
          aria-label="插入表格"
          v-tooltip="'插入表格（3×3）'"
          @click="
            run((e) =>
              e
                .chain()
                .focus()
                .insertTable({ rows: 3, cols: 3, withHeaderRow: true })
                .run(),
            )
          "
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_TABLE" /></svg>
        </button>
        <button
          class="docx-editor-tool-btn"
          aria-label="插入图片"
          v-tooltip="'插入图片'"
          @click="imageInput?.click()"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_IMAGE" /></svg>
        </button>
        <button
          class="docx-editor-tool-btn"
          :class="{ active: isActive('link') }"
          aria-label="链接"
          v-tooltip="'链接'"
          @click="onInsertLink"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_LINK" /></svg>
        </button>
        <input
          ref="imageInput"
          type="file"
          accept="image/*"
          hidden
          @change="onImageFile"
        />
        <span class="docx-editor-tool-spacer" />
        <button
          class="docx-editor-tool-btn primary"
          :disabled="!tab.dirty || tab.saving"
          aria-label="保存"
          v-tooltip="'保存（Ctrl+S）'"
          @click="saveDocxTab(tab.id)"
        >
          <svg viewBox="0 0 24 24"><path :d="ICON_SAVE" /></svg>
        </button>
      </div>
      <div class="docx-editor-host">
        <EditorContent
          :key="tab.id"
          :editor="tab.editor ?? undefined"
        />
      </div>
    </div>
    <div class="docx-editor-status">
      <span v-if="tab.dirty">未保存</span>
      <span>{{ byteSizeLabel }}</span>
      <span>{{ tab.status }}</span>
      <span class="docx-editor-status-hint">
        保存后页眉页脚、批注、修订等高级格式可能丢失
      </span>
    </div>
  </div>
</template>
