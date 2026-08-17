/** .xlsx 只读预览的解析层：字节 → 可渲染的工作表网格（SheetJS 社区版） */

import * as XLSX from "xlsx";

/** 单个工作表的展示数据：全部单元格为格式化文本，行已补齐到表宽 */
export interface XlsxSheetData {
  name: string;
  /** 单元格格式化文本（含日期/数字格式），未使用单元格为空串 */
  rows: string[][];
  rowCount: number;
  colCount: number;
  /** 列宽（px）：优先取 !cols 元数据（wch 字符宽换算），缺省 120 */
  colWidths: number[];
}

export interface XlsxWorkbook {
  sheetNames: string[];
  sheets: XlsxSheetData[];
}

const DEFAULT_COL_WIDTH = 120;
const MIN_COL_WIDTH = 64;
const MAX_COL_WIDTH = 320;

/** 单元格文本化：sheet_to_json raw:false 输出格式化文本或原始值，统一转字符串 */
function cellText(v: unknown): string {
  if (v === null || v === undefined) return "";
  return String(v);
}

/** 列宽元数据 → px：wch 为字符数，约 7px/字符 + 内边距；宽度类按原样夹紧 */
function colWidthPx(w: XLSX.ColInfo | undefined): number {
  if (!w) return DEFAULT_COL_WIDTH;
  const raw = typeof w.wch === "number" ? Math.round(w.wch * 7) + 12 : w.width;
  if (typeof raw !== "number" || !Number.isFinite(raw) || raw <= 0) {
    return DEFAULT_COL_WIDTH;
  }
  return Math.min(MAX_COL_WIDTH, Math.max(MIN_COL_WIDTH, Math.round(raw)));
}

/**
 * .xlsx 原始字节 → 工作簿展示模型。
 * 每个工作表经 sheet_to_json（header:1、raw:false 取格式化文本、defval 空串）
 * 转为行数组，并补齐到 !ref 声明的表宽；合并范围覆盖的单元格置空（值仅显示
 * 在左上角）。非法/非 .xlsx 字节或空工作簿抛出错误。
 */
export function parseXlsx(bytes: Uint8Array): XlsxWorkbook {
  // .xlsx 为 ZIP 容器（PK\x03\x04 / 空包 PK\x05\x06 / 跨卷 PK\x07\x08）：
  // 文件头校验优先于 SheetJS 兜底解析，避免垃圾字节被当作 AOA 静默放行
  const isZipMagic =
    bytes.length >= 4 &&
    bytes[0] === 0x50 &&
    bytes[1] === 0x4b &&
    (bytes[2] === 0x03 || bytes[2] === 0x05 || bytes[2] === 0x07) &&
    (bytes[3] === 0x04 || bytes[3] === 0x06 || bytes[3] === 0x08);
  if (!isZipMagic) {
    throw new Error("不是有效的 .xlsx 文件（无法识别文件头）");
  }
  // cellStyles 开启才能读到 !cols（列宽）与 !merges（合并范围）
  const wb = XLSX.read(bytes, { type: "array", cellStyles: true });
  if (wb.SheetNames.length === 0) {
    throw new Error("不是有效的 .xlsx 文件（未包含工作表）");
  }
  const sheets: XlsxSheetData[] = wb.SheetNames.map((name) => {
    const ws = wb.Sheets[name];
    let colCount = 0;
    let rows: string[][] = [];
    const ref = ws["!ref"];
    if (ref) {
      const range = XLSX.utils.decode_range(ref);
      colCount = range.e.c + 1;
      // 合并范围覆盖的单元格：除左上角外一律置空
      const covered = new Set<string>();
      for (const m of ws["!merges"] ?? []) {
        for (let r = m.s.r; r <= m.e.r; r++) {
          for (let c = m.s.c; c <= m.e.c; c++) {
            if (r !== m.s.r || c !== m.s.c) covered.add(`${r}:${c}`);
          }
        }
      }
      const rawRows = XLSX.utils.sheet_to_json(ws, {
        header: 1,
        raw: false,
        defval: "",
      }) as unknown[][];
      rows = rawRows.map((row, r) => {
        const out = new Array<string>(colCount).fill("");
        for (let c = 0; c < Math.min(row.length, colCount); c++) {
          out[c] = covered.has(`${range.s.r + r}:${range.s.c + c}`)
            ? ""
            : cellText(row[c]);
        }
        return out;
      });
    }
    const colWidths = Array.from({ length: colCount }, (_, c) =>
      colWidthPx(ws["!cols"]?.[c]),
    );
    return { name, rows, rowCount: rows.length, colCount, colWidths };
  });
  return { sheetNames: wb.SheetNames, sheets };
}
