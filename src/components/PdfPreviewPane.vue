<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import * as pdfjsLib from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { nextTick } from "vue";
import { useCtrlWheelZoom } from "../composables/useCtrlWheelZoom";
import type { PreviewEditorTab } from "../composables/useEditorTabs";

pdfjsLib.GlobalWorkerOptions.workerSrc = workerUrl;

const props = defineProps<{
  tab: PreviewEditorTab;
  /** 头部动作区容器：工具栏 Teleport 目标；缺省时原位渲染 */
  actionsTarget?: HTMLElement | null;
}>();

const MIN_SCALE = 0.25;
const MAX_SCALE = 4;
const SCALE_STEP = 1.25;
/** 画布相对容器两侧的留白（px） */
const CANVAS_PADDING = 24;

const loading = ref(true);
const error = ref("");
const pageCount = ref(0);
const pageNo = ref(1);
/** 缩放倍率：1 表示适应容器宽度 */
const zoom = ref(1);
const canvasHost = ref<HTMLDivElement | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);

let doc: pdfjsLib.PDFDocumentProxy | null = null;
let loadingTask: pdfjsLib.PDFDocumentLoadingTask | null = null;
let renderTask: pdfjsLib.RenderTask | null = null;
let renderSeq = 0;

useCtrlWheelZoom(canvasHost, zoom, MIN_SCALE, MAX_SCALE, SCALE_STEP);

const percentLabel = computed(() => `${Math.round(zoom.value * 100)}%`);

async function load() {
  loading.value = true;
  error.value = "";
  renderTask?.cancel();
  renderTask = null;
  loadingTask?.destroy().catch(() => {});
  loadingTask = null;
  doc = null;
  // 外部刷新（pdfData 替换）时保留当前页与缩放，加载完成后夹紧恢复
  const prevPage = pageNo.value;
  const prevZoom = zoom.value;
  pageCount.value = 0;
  pageNo.value = 1;
  zoom.value = 1;
  const data = props.tab.pdfData;
  if (!data || !data.length) {
    error.value = "PDF 内容为空";
    loading.value = false;
    return;
  }
  try {
    // 传副本给 pdfjs：其内部会经 structuredClone(transfer) 把 data.buffer 转移给
    // worker，直接传 props.tab.pdfData 会把标签上保存的字节 detach 成空 buffer，
    // 导致切标签重挂载后 load() 读到空字节而误报「PDF 内容为空」。
    const task = pdfjsLib.getDocument({ data: data.slice() });
    loadingTask = task;
    const d = await task.promise;
    doc = d;
    pageCount.value = d.numPages;
    pageNo.value = Math.min(prevPage, d.numPages);
    zoom.value = prevZoom;
    props.tab.pageCount = d.numPages;
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
  if (!doc) return; // 加载失败：错误信息已在 error 中展示
  await nextTick(); // 等待 canvas 挂载后再绘制
  await renderPage();
}

/** 渲染当前页到 canvas：以容器宽度为基准适配，乘以用户缩放倍率 */
async function renderPage() {
  if (!doc) return;
  const seq = ++renderSeq;
  const page = await doc.getPage(pageNo.value);
  if (seq !== renderSeq) return;
  const host = canvasHost.value;
  const c = canvas.value;
  if (!host || !c) return;
  const base = page.getViewport({ scale: 1 });
  const fit = Math.max(0.1, (host.clientWidth - CANVAS_PADDING) / base.width);
  const viewport = page.getViewport({ scale: fit * zoom.value });
  c.width = Math.floor(viewport.width);
  c.height = Math.floor(viewport.height);
  c.style.width = `${Math.floor(viewport.width)}px`;
  c.style.height = `${Math.floor(viewport.height)}px`;
  const ctx = c.getContext("2d");
  if (!ctx) {
    error.value = "当前环境不支持画布渲染";
    return;
  }
  renderTask?.cancel();
  const task = page.render({ canvas: c, viewport });
  renderTask = task;
  try {
    await task.promise;
  } finally {
    if (renderTask === task) renderTask = null;
  }
}

function prevPage() {
  if (pageNo.value > 1) pageNo.value--;
}

function nextPage() {
  if (pageNo.value < pageCount.value) pageNo.value++;
}

function zoomIn() {
  zoom.value = Math.min(MAX_SCALE, zoom.value * SCALE_STEP);
}

function zoomOut() {
  zoom.value = Math.max(MIN_SCALE, zoom.value / SCALE_STEP);
}

function fitWidth() {
  zoom.value = 1;
}

// 切换标签或外部刷新替换 pdfData：重新加载文档；页码与缩放变化：重绘当前页
watch(
  [() => props.tab, () => props.tab.pdfData],
  () => void load(),
  { immediate: true },
);
watch([pageNo, zoom], () => void renderPage());

onBeforeUnmount(() => {
  renderTask?.cancel();
  renderTask = null;
  loadingTask?.destroy().catch(() => {});
  loadingTask = null;
});
</script>

<template>
  <div class="pdf-preview">
    <Teleport :to="props.actionsTarget" :disabled="!props.actionsTarget">
      <div class="pdf-toolbar">
        <button
          class="pdf-toolbar-btn"
          :disabled="pageNo <= 1"
          aria-label="上一页"
          v-tooltip="'上一页'"
          @click="prevPage()"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M15.41 7.41 14 6l-6 6 6 6 1.41-1.41L10.83 12z" />
          </svg>
        </button>
        <button
          class="pdf-toolbar-btn"
          :disabled="pageNo >= pageCount"
          aria-label="下一页"
          v-tooltip="'下一页'"
          @click="nextPage()"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M8.59 16.59 10 18l6-6-6-6-1.41 1.41L13.17 12z" />
          </svg>
        </button>
        <span class="pdf-toolbar-page">{{ pageNo }} / {{ pageCount }}</span>
        <span class="pdf-toolbar-sep"></span>
        <button
          class="pdf-toolbar-btn"
          :disabled="zoom <= MIN_SCALE"
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
          :disabled="zoom >= MAX_SCALE"
          aria-label="放大"
          v-tooltip="'放大'"
          @click="zoomIn()"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M11 5h2v6h6v2h-6v6h-2v-6H5v-2h6z" />
          </svg>
        </button>
        <button class="pdf-toolbar-btn pdf-toolbar-fit" @click="fitWidth()">
          适应宽度
        </button>
      </div>
    </Teleport>
    <div v-if="loading" class="preview-note">正在渲染 PDF…</div>
    <div v-else-if="error" class="preview-note preview-error">
      无法预览该 PDF（{{ error }}）
    </div>
    <div v-else ref="canvasHost" class="pdf-canvas-host">
      <canvas ref="canvas"></canvas>
    </div>
  </div>
</template>
