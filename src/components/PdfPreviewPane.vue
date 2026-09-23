<script setup lang="ts">
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  ref,
  watch,
} from "vue";
import * as pdfjsLib from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
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
/** 视口外预渲染边距（px）：滚动到附近提前渲染，避免滚动等待 */
const RENDER_MARGIN = 600;

const loading = ref(true);
const error = ref("");
const pageCount = ref(0);
const pageNo = ref(1);
/** 缩放倍率：1 表示适应容器宽度 */
const zoom = ref(1);
const canvasHost = ref<HTMLDivElement | null>(null);

/** 每页基础尺寸（scale=1）：占位高度与统一缩放的依据 */
interface PageBaseDims {
  width: number;
  height: number;
}
const pageDims = ref<PageBaseDims[]>([]);
/** 适应宽度基准比例：全部页面统一，取最宽页计算（随容器宽度重算） */
const baseFit = ref(1);
const currentScale = computed(() => baseFit.value * zoom.value);

// 每页渲染状态（非响应式：canvas/文本层容器经模板函数 ref 收集）
const pageCanvases = new Map<number, HTMLCanvasElement>();
const textHosts = new Map<number, HTMLDivElement>();
const renderTasks = new Map<number, pdfjsLib.RenderTask>();
const textLayers = new Map<number, pdfjsLib.TextLayer>();
/** 已完成渲染的页 → 渲染比例：同比例重复请求直接跳过 */
const renderedScale = new Map<number, number>();
let io: IntersectionObserver | null = null;
let scrollRaf: number | undefined;

function setPageCanvas(n: number, el: unknown) {
  if (el instanceof HTMLCanvasElement) pageCanvases.set(n, el);
  else pageCanvases.delete(n);
}

function setTextHost(n: number, el: unknown) {
  if (el instanceof HTMLDivElement) textHosts.set(n, el);
  else textHosts.delete(n);
}

function cancelAllRenderTasks() {
  for (const t of renderTasks.values()) t.cancel();
  renderTasks.clear();
  for (const l of textLayers.values()) l.cancel();
  textLayers.clear();
  renderedScale.clear();
}

useCtrlWheelZoom(canvasHost, zoom, MIN_SCALE, MAX_SCALE, SCALE_STEP);

const percentLabel = computed(() => `${Math.round(zoom.value * 100)}%`);

/** 页面占位尺寸：未渲染前即按统一比例占位，保证滚动位置稳定 */
function pageStyle(n: number) {
  const dims = pageDims.value[n - 1];
  if (!dims) return undefined;
  const s = currentScale.value;
  return {
    width: `${Math.floor(dims.width * s)}px`,
    height: `${Math.floor(dims.height * s)}px`,
  };
}

function recalcFit() {
  const host = canvasHost.value;
  if (!host || !pageDims.value.length) return;
  const maxW = Math.max(...pageDims.value.map((p) => p.width));
  baseFit.value = Math.max(0.1, (host.clientWidth - CANVAS_PADDING) / maxW);
}

async function load() {
  loading.value = true;
  error.value = "";
  cancelAllRenderTasks();
  io?.disconnect();
  io = null;
  loadingTask?.destroy().catch(() => {});
  loadingTask = null;
  doc = null;
  // 外部刷新（pdfData 替换）时保留当前页与缩放，加载完成后夹紧恢复并滚回该页
  const prevPage = pageNo.value;
  const prevZoom = zoom.value;
  pageCount.value = 0;
  pageNo.value = 1;
  zoom.value = 1;
  pageDims.value = [];
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
    props.tab.pageCount = d.numPages;
    // 预取每页基础尺寸：占位平铺与统一适应宽度都需要
    const dims: PageBaseDims[] = [];
    for (let n = 1; n <= d.numPages; n++) {
      const p = await d.getPage(n);
      const v = p.getViewport({ scale: 1 });
      dims.push({ width: v.width, height: v.height });
    }
    pageDims.value = dims;
    pageNo.value = Math.min(prevPage, d.numPages);
    zoom.value = prevZoom;
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
  if (!doc) return; // 加载失败：错误信息已在 error 中展示
  await nextTick(); // 等待全部页面占位挂载
  recalcFit();
  setupObserver();
  scrollToPage(pageNo.value);
  renderVisiblePages();
}

let doc: pdfjsLib.PDFDocumentProxy | null = null;
let loadingTask: pdfjsLib.PDFDocumentLoadingTask | null = null;

/** 渲染单页（画布 + 文本层）：同比例已渲染则跳过；按页独立取消，滚动浏览互不干扰 */
async function renderPage(n: number) {
  const d = doc;
  if (!d) return;
  const s = currentScale.value;
  if (renderedScale.get(n) === s) return;
  const canvas = pageCanvases.get(n);
  if (!canvas) return;
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    error.value = "当前环境不支持画布渲染";
    return;
  }
  const page = await d.getPage(n);
  const viewport = page.getViewport({ scale: s });
  canvas.width = Math.floor(viewport.width);
  canvas.height = Math.floor(viewport.height);
  renderTasks.get(n)?.cancel();
  const task = page.render({ canvas, viewport });
  renderTasks.set(n, task);
  try {
    await task.promise;
  } catch {
    return; // 渲染取消/失败：保留占位，滚动重新进入时重试
  }
  if (renderTasks.get(n) === task) renderTasks.delete(n);
  renderedScale.set(n, s);
  await buildTextLayer(n, page, viewport);
}

/** 重建单页文本选择层：透明文字按视口排版覆盖在画布上，供鼠标选择复制 */
async function buildTextLayer(
  n: number,
  page: pdfjsLib.PDFPageProxy,
  viewport: pdfjsLib.PageViewport,
) {
  const host = textHosts.get(n);
  if (!host) return;
  textLayers.get(n)?.cancel();
  textLayers.delete(n);
  host.replaceChildren();
  // pdf.js 文本层排版依赖 --total-scale-factor 计算字号与变换
  host.style.setProperty("--total-scale-factor", String(viewport.scale));
  let content: Awaited<
    ReturnType<pdfjsLib.PDFPageProxy["getTextContent"]>
  > | null = null;
  try {
    content = await page.getTextContent();
  } catch {
    return; // 文本内容读取失败：仅影响选择复制
  }
  const layer = new pdfjsLib.TextLayer({
    textContentSource: content,
    container: host,
    viewport,
  });
  textLayers.set(n, layer);
  try {
    await layer.render();
  } catch {
    // 渲染取消/失败静默：选择复制不可用即可，不影响画布
  } finally {
    if (textLayers.get(n) === layer) textLayers.delete(n);
  }
}

/** 懒渲染：页面滚入视口（含预渲染边距）时触发；无 IO 环境（测试/旧内核）全量兜底 */
function setupObserver() {
  io?.disconnect();
  const host = canvasHost.value;
  if (!host) return;
  if (typeof IntersectionObserver === "undefined") {
    for (let n = 1; n <= pageCount.value; n++) void renderPage(n);
    return;
  }
  io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        const n = Number((e.target as HTMLElement).dataset.page);
        if (n) void renderPage(n);
      }
    },
    { root: host, rootMargin: `${RENDER_MARGIN}px 0px` },
  );
  for (const el of host.querySelectorAll(".pdf-page-wrap")) io.observe(el);
}

/** 显式渲染视口附近页面：缩放/容器尺寸变化后调用（IO 仅在交叉状态变化时触发） */
function renderVisiblePages() {
  const host = canvasHost.value;
  if (!host) return;
  const hostRect = host.getBoundingClientRect();
  for (let n = 1; n <= pageCount.value; n++) {
    const el = host.querySelector<HTMLElement>(
      `.pdf-page-wrap[data-page="${n}"]`,
    );
    if (!el) continue;
    const r = el.getBoundingClientRect();
    // 未占位/零尺寸（无布局环境）视为不可见，跳过
    if (r.width === 0 && r.height === 0) continue;
    if (r.bottom < hostRect.top - RENDER_MARGIN) continue;
    if (r.top > hostRect.bottom + RENDER_MARGIN) continue;
    void renderPage(n);
  }
}

/** 滚动定位到指定页顶部（恢复阅读位置 / 上一页下一页按钮共用） */
function scrollToPage(n: number) {
  const host = canvasHost.value;
  if (!host || typeof host.scrollTo !== "function") return;
  const el = host.querySelector<HTMLElement>(
    `.pdf-page-wrap[data-page="${n}"]`,
  );
  if (!el) return;
  try {
    host.scrollTo({ top: Math.max(0, el.offsetTop - 8) });
  } catch {
    // jsdom 等无滚动实现环境忽略
  }
}

// 滚动跟随页码：取视口垂直中点所在的页，rAF 合并高频滚动
function onHostScroll() {
  if (scrollRaf !== undefined) return;
  scrollRaf = requestAnimationFrame(() => {
    scrollRaf = undefined;
    const host = canvasHost.value;
    if (!host) return;
    const hostRect = host.getBoundingClientRect();
    const mid = hostRect.top + hostRect.height / 2;
    let cur = 1;
    for (let n = 1; n <= pageCount.value; n++) {
      const el = host.querySelector<HTMLElement>(
        `.pdf-page-wrap[data-page="${n}"]`,
      );
      if (!el) continue;
      if (el.getBoundingClientRect().top <= mid) cur = n;
      else break;
    }
    if (cur !== pageNo.value) pageNo.value = cur;
  });
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

// 切换标签或外部刷新替换 pdfData：重新加载文档；缩放变化：重渲染视口附近页面
watch([() => props.tab, () => props.tab.pdfData], () => void load(), {
  immediate: true,
});
watch(zoom, () => renderVisiblePages());

// 窗口尺寸变化：适应宽度基准随容器宽度重算并重渲染
function onWindowResize() {
  recalcFit();
  renderVisiblePages();
}

onMounted(() => {
  window.addEventListener("resize", onWindowResize);
});

onBeforeUnmount(() => {
  window.removeEventListener("resize", onWindowResize);
  cancelAllRenderTasks();
  io?.disconnect();
  io = null;
  if (scrollRaf !== undefined) cancelAnimationFrame(scrollRaf);
  scrollRaf = undefined;
  loadingTask?.destroy().catch(() => {});
  loadingTask = null;
});
</script>

<template>
  <div class="pdf-preview">
    <Teleport :to="props.actionsTarget" :disabled="!props.actionsTarget">
      <div class="pdf-toolbar">
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
    <div
      v-else
      ref="canvasHost"
      class="pdf-canvas-host"
      @scroll.passive="onHostScroll"
    >
      <div
        v-for="n in pageCount"
        :key="n"
        class="pdf-page-wrap"
        :data-page="n"
        :style="pageStyle(n)"
      >
        <canvas :ref="(el) => setPageCanvas(n, el)"></canvas>
        <div class="pdf-text-layer" :ref="(el) => setTextHost(n, el)"></div>
      </div>
    </div>
  </div>
</template>
