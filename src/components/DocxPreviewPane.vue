<script setup lang="ts">
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  ref,
  watch,
} from "vue";
import { useCtrlWheelZoom } from "../composables/useCtrlWheelZoom";
import type { PreviewEditorTab } from "../composables/useEditorTabs";

const props = defineProps<{
  tab: PreviewEditorTab;
  /** 头部动作区容器：工具栏 Teleport 目标；缺省时原位渲染 */
  actionsTarget?: HTMLElement | null;
}>();

const MIN_ZOOM = 0.25;
const MAX_ZOOM = 4;
const ZOOM_STEP = 1.25;
/** 滚动容器两侧留白（px），适应宽度时扣除 */
const STAGE_PADDING = 24;

const loading = ref(true);
const error = ref("");
/** 滚动容器：docx-preview 把分页 DOM 注入其中的 sizer */
const stage = ref<HTMLDivElement | null>(null);
/** 渲染容器（zoom 缩放作用于此）：width:max-content + margin:auto 安全居中 */
const sizer = ref<HTMLDivElement | null>(null);
const zoom = ref(1);
/** 渲染序号：外部刷新连发时丢弃过期渲染结果 */
let renderSeq = 0;
/** 首次渲染后自动适应宽度一次；此后保留用户缩放 */
let autoFitDone = false;

useCtrlWheelZoom(stage, zoom, MIN_ZOOM, MAX_ZOOM, ZOOM_STEP);

const percentLabel = computed(() => `${Math.round(zoom.value * 100)}%`);

function zoomIn() {
  zoom.value = Math.min(MAX_ZOOM, zoom.value * ZOOM_STEP);
}

function zoomOut() {
  zoom.value = Math.max(MIN_ZOOM, zoom.value / ZOOM_STEP);
}

/** 适应宽度：页面自然宽（section + wrapper 左右 padding）与容器宽求比值 */
function fitWidth() {
  const host = stage.value;
  const page = host?.querySelector("section");
  if (!host || !page) return;
  const wrapStyle = getComputedStyle(page.parentElement ?? page);
  const pad =
    (parseFloat(wrapStyle.paddingLeft) || 0) +
    (parseFloat(wrapStyle.paddingRight) || 0);
  const natural = page.offsetWidth + pad;
  if (natural <= 0 || host.clientWidth <= 0) return;
  zoom.value = Math.min(
    MAX_ZOOM,
    Math.max(MIN_ZOOM, (host.clientWidth - STAGE_PADDING * 2) / natural),
  );
}

async function load() {
  const seq = ++renderSeq;
  loading.value = true;
  error.value = "";
  const data = props.tab.docxData;
  if (!data || !data.length) {
    error.value = "文档内容为空";
    loading.value = false;
    return;
  }
  // 外部刷新替换字节时保留滚动位置（与文本/PDF 刷新的恢复策略一致）
  const prevScroll = stage.value?.scrollTop ?? 0;
  try {
    // 传副本给 docx-preview：隔离标签字节与渲染库的潜在改动，
    // 保证切标签重挂载后 load() 仍能从 tab.docxData 完整重载
    const docx = await import("docx-preview");
    if (seq !== renderSeq || !sizer.value) return;
    sizer.value.innerHTML = "";
    await docx.renderAsync(
      data.slice(),
      sizer.value,
      undefined,
      {
        inWrapper: true,
        breakPages: true,
        renderHeaders: true,
        renderFooters: true,
        ignoreLastRenderedPageBreak: false,
        // 嵌入图片/字体改为 base64 data URL：应用 CSP 的 img-src 未放行 blob:，
        // 而 docx-preview 默认用 URL.createObjectURL → blob: 会被拦截致图片加载失败；
        // data: 已在 img-src/font-src 中放行（useBase64URL 为 docx-preview 专为此场景提供的选项）。
        useBase64URL: true,
      },
    );
    if (seq !== renderSeq) return;
    // 先解除隐藏态再测量（v-show 隐藏时无布局，适应宽度量不到尺寸）
    loading.value = false;
    await nextTick();
    if (!autoFitDone) {
      autoFitDone = true;
      fitWidth();
    }
    if (stage.value) stage.value.scrollTop = prevScroll;
  } catch (e) {
    if (seq !== renderSeq) return;
    error.value = String(e);
  } finally {
    if (seq === renderSeq) loading.value = false;
  }
}

// 挂载后首次渲染（此时 stage 已挂载、docxData 已就绪）；
// 切换预览标签或外部刷新替换 docxData：重新渲染文档
onMounted(() => void load());
watch(
  [() => props.tab, () => props.tab.docxData],
  () => void load(),
);

onBeforeUnmount(() => {
  renderSeq++;
});
</script>

<template>
  <div class="docx-preview">
    <Teleport :to="props.actionsTarget" :disabled="!props.actionsTarget">
      <div class="pdf-toolbar">
        <button
          class="pdf-toolbar-btn"
          :disabled="zoom <= MIN_ZOOM"
          aria-label="缩小"
          v-tooltip="'缩小'"
          @click="zoomOut()"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M7 11h10v2H7z" />
          </svg>
        </button>
        <span class="pdf-toolbar-percent">{{ percentLabel }}</span>
        <button
          class="pdf-toolbar-btn"
          :disabled="zoom >= MAX_ZOOM"
          aria-label="放大"
          v-tooltip="'放大'"
          @click="zoomIn()"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M11 5h2v6h6v2h-6v6h-2v-6H5v-2h6z" />
          </svg>
        </button>
        <button
          class="pdf-toolbar-btn pdf-toolbar-fit"
          v-tooltip="'缩放到页面宽度'"
          @click="fitWidth()"
        >
          适应宽度
        </button>
      </div>
    </Teleport>
    <div v-if="loading" class="preview-note">正在渲染文档…</div>
    <div v-else-if="error" class="preview-note preview-error">
      无法预览该文档（{{ error }}）
    </div>
    <div v-show="!loading && !error" ref="stage" class="docx-preview-stage">
      <div ref="sizer" class="docx-preview-sizer" :style="{ zoom }"></div>
    </div>
  </div>
</template>
