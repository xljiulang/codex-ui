<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { useCtrlWheelZoom } from "../composables/useCtrlWheelZoom";
import type { PreviewEditorTab } from "../composables/useEditorTabs";
import { extOf } from "../lib/preview";
import { parseXlsx, type XlsxSheetData, type XlsxWorkbook } from "../lib/xlsx";

const props = defineProps<{
  tab: PreviewEditorTab;
  /** 头部动作区容器：工具栏 Teleport 目标；缺省时原位渲染 */
  actionsTarget?: HTMLElement | null;
}>();

/** 网格固定行高（px）：虚拟滚动的行高基准 */
const ROW_HEIGHT = 28;
/** 可视区上下额外渲染的行数，避免快速滚动露出空白 */
const OVERSCAN = 10;
/** 缩放范围与步进：0.5–3 倍，与 PDF 预览同一套交互 */
const MIN_ZOOM = 0.5;
const MAX_ZOOM = 3;
const ZOOM_STEP = 1.25;

const loading = ref(true);
const error = ref("");
const workbook = ref<XlsxWorkbook | null>(null);
const sheetIndex = ref(0);
const zoom = ref(1);
const gridHost = ref<HTMLDivElement | null>(null);
const startRow = ref(0);
const endRow = ref(0);

useCtrlWheelZoom(gridHost, zoom, MIN_ZOOM, MAX_ZOOM, ZOOM_STEP);

const activeSheet = computed<XlsxSheetData | null>(() => {
  const wb = workbook.value;
  if (!wb || wb.sheets.length === 0) return null;
  const i = Math.min(Math.max(0, sheetIndex.value), wb.sheets.length - 1);
  return wb.sheets[i];
});

/** 当前可见行切片：随滚动窗口变化，由 startRow/endRow 决定 */
const visibleRows = computed(() => {
  const sheet = activeSheet.value;
  if (!sheet) return [];
  return sheet.rows.slice(startRow.value, endRow.value);
});

const percentLabel = computed(() => `${Math.round(zoom.value * 100)}%`);
/** 缩放后的行高（px）：虚拟滚动与单元格高度共用 */
const rowHeightPx = computed(() => Math.round(ROW_HEIGHT * zoom.value));
/** 缩放后的字号（px）：内容随缩放同步放大 */
const fontPx = computed(() => Math.round(12 * zoom.value));
/** 缩放后的列宽（px） */
const scaledWidths = computed(() => {
  const sheet = activeSheet.value;
  if (!sheet) return [];
  return sheet.colWidths.map((w) => Math.round(w * zoom.value));
});

/** 表格最小宽度：缩放后列宽之和，窄于容器时由 100% 撑满 */
const tableWidth = computed(() => {
  return scaledWidths.value.reduce((a, b) => a + b, 0);
});

/** 滚动窗口计算：按容器滚动位置确定可见行区间（含上下 overscan） */
function updateWindow() {
  const host = gridHost.value;
  const sheet = activeSheet.value;
  if (!host || !sheet || sheet.rowCount === 0) return;
  const rowH = rowHeightPx.value;
  const first = Math.max(0, Math.floor(host.scrollTop / rowH) - OVERSCAN);
  const count = Math.ceil(host.clientHeight / rowH) + OVERSCAN * 2;
  startRow.value = first;
  endRow.value = Math.min(sheet.rowCount, first + count);
}

function onScroll() {
  updateWindow();
}

function zoomIn() {
  zoom.value = Math.min(MAX_ZOOM, zoom.value * ZOOM_STEP);
}

function zoomOut() {
  zoom.value = Math.max(MIN_ZOOM, zoom.value / ZOOM_STEP);
}

function resetZoom() {
  zoom.value = 1;
}

function selectSheet(i: number) {
  if (i === sheetIndex.value) return;
  sheetIndex.value = i;
  props.tab.xlsxSheetIndex = i;
  if (gridHost.value) gridHost.value.scrollTop = 0;
  updateWindow();
}

/** 列标题：A…Z、AA…（与 Excel 列号一致） */
function colLabel(c: number): string {
  let s = "";
  let n = c;
  do {
    s = String.fromCharCode(65 + (n % 26)) + s;
    n = Math.floor(n / 26) - 1;
  } while (n >= 0);
  return s;
}

/** 外部刷新替换 xlsxData 或切换标签后重解析；保留当前表序号并夹紧 */
async function load() {
  loading.value = true;
  error.value = "";
  workbook.value = null;
  const bytes = props.tab.xlsxData;
  if (!bytes || bytes.length === 0) {
    error.value = "表格内容为空";
    loading.value = false;
    return;
  }
  await nextTick(); // 先渲染 loading 态，再同步解析（大表避免无反馈卡顿）
  try {
    const wb = parseXlsx(bytes, extOf(props.tab.path) ?? "xlsx");
    workbook.value = wb;
    if (wb.sheets.length > 0) {
      const saved = props.tab.xlsxSheetIndex;
      sheetIndex.value = Math.min(
        Math.max(0, Number.isFinite(saved) ? saved : 0),
        wb.sheets.length - 1,
      );
    }
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
  await nextTick();
  updateWindow();
}

watch([() => props.tab, () => props.tab.xlsxData], () => void load(), {
  immediate: true,
});

// 缩放变化后行高/列宽重新布局，浏览器会夹紧滚动位置：下一帧重算可见窗口
watch(zoom, () => {
  void nextTick(updateWindow);
});
</script>

<template>
  <div class="xlsx-preview">
    <Teleport :to="props.actionsTarget" :disabled="!props.actionsTarget">
      <div class="xlsx-toolbar">
        <div class="xlsx-sheets">
          <button
            v-for="(sheet, i) in workbook?.sheets ?? []"
            :key="sheet.name"
            class="xlsx-sheet-btn"
            :class="{ active: i === sheetIndex }"
            type="button"
            @click="selectSheet(i)"
          >
            {{ sheet.name }}
          </button>
        </div>
        <div class="xlsx-zoom">
          <button
            class="preview-zoom-btn"
            type="button"
            :disabled="zoom <= MIN_ZOOM"
            aria-label="缩小"
            @click="zoomOut()"
          >
            −
          </button>
          <span class="preview-zoom-percent">{{ percentLabel }}</span>
          <button
            class="preview-zoom-btn"
            type="button"
            :disabled="zoom >= MAX_ZOOM"
            aria-label="放大"
            @click="zoomIn()"
          >
            ＋
          </button>
          <button class="preview-zoom-btn" type="button" @click="resetZoom()">
            100%
          </button>
        </div>
        <span v-if="activeSheet" class="xlsx-meta">
          {{ activeSheet.rowCount }} 行 × {{ activeSheet.colCount }} 列
        </span>
      </div>
    </Teleport>
    <div v-if="loading" class="preview-note">正在解析表格…</div>
    <div v-else-if="error" class="preview-note preview-error">
      无法预览该表格（{{ error }}）
    </div>
    <div v-else-if="!activeSheet" class="preview-note">该工作簿没有工作表</div>
    <div v-else-if="activeSheet.rowCount === 0" class="preview-note">
      该工作表为空
    </div>
    <div v-else ref="gridHost" class="xlsx-grid-host" @scroll="onScroll">
      <table
        class="xlsx-table"
        :style="{
          minWidth: `${tableWidth}px`,
          fontSize: `${fontPx}px`,
          '--xlsx-row-h': `${rowHeightPx}px`,
        }"
      >
        <colgroup>
          <col
            v-for="(w, c) in scaledWidths"
            :key="c"
            :style="{ width: `${w}px` }"
          />
        </colgroup>
        <thead>
          <tr>
            <th v-for="c in activeSheet.colCount" :key="c" class="xlsx-th">
              {{ colLabel(c - 1) }}
            </th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="startRow > 0" class="xlsx-spacer">
            <td
              :colspan="activeSheet.colCount"
              :style="{ height: `${startRow * rowHeightPx}px` }"
            />
          </tr>
          <tr v-for="(row, i) in visibleRows" :key="startRow + i">
            <td
              v-for="(cell, c) in row"
              :key="c"
              class="xlsx-td"
              v-tooltip="cell"
            >
              {{ cell }}
            </td>
          </tr>
          <tr v-if="endRow < activeSheet.rowCount" class="xlsx-spacer">
            <td
              :colspan="activeSheet.colCount"
              :style="{
                height: `${(activeSheet.rowCount - endRow) * rowHeightPx}px`,
              }"
            />
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>
