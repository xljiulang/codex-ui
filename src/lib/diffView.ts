/**
 * diff 视图的渲染预算（纯函数 + 常量，供 DiffPane 与单测共用）。
 *
 * 内联 diff 有两个「与行数不成正比」的开销：
 * 1. 行内语法高亮按行做，压缩成一行的大 JSON 只有 2~4 行却可能是 MB 级文本
 *    （实测 highlight.js 高亮 3 MB 单行需约 3.2 s、产出 21 MB HTML）；
 * 2. 每行渲染 3~4 个 DOM 节点，几万行时会一次性挂上十万级节点。
 * 因此高亮按「字符量」设预算，渲染按「行数」设上限。
 */

import type { DiffRow } from "./types";

/** 单行高亮预算：单行文本超过该长度时整份 diff 都不高亮 */
export const DIFF_MAX_HIGHLIGHT_LINE_CHARS = 4_000;
/** 整份 diff 的高亮预算（各行文本长度之和） */
export const DIFF_MAX_HIGHLIGHT_CHARS = 200_000;
/** 参与高亮的最大行数（超过则整份 diff 回退纯文本） */
export const DIFF_MAX_HIGHLIGHT_LINES = 20_000;
/** 单次最多渲染的行数（超出截断并提示，避免十万级 DOM 节点） */
export const DIFF_MAX_RENDER_ROWS = 10_000;

/** 整份 diff 是否适合逐行语法高亮（无语言、行数或字符量超预算时为 false） */
export function canHighlightDiffRows(
  hasLanguage: boolean,
  rows: readonly DiffRow[],
): boolean {
  if (!hasLanguage || rows.length > DIFF_MAX_HIGHLIGHT_LINES) return false;
  let total = 0;
  for (const row of rows) {
    if (row.kind === "sep") continue;
    if (row.text.length > DIFF_MAX_HIGHLIGHT_LINE_CHARS) return false;
    total += row.text.length;
    if (total > DIFF_MAX_HIGHLIGHT_CHARS) return false;
  }
  return true;
}

/** 按渲染上限裁剪行数组：返回可见行与被省略的行数 */
export function sliceDiffRows<T>(rows: T[]): { visible: T[]; truncated: number } {
  if (rows.length <= DIFF_MAX_RENDER_ROWS) {
    return { visible: rows, truncated: 0 };
  }
  return {
    visible: rows.slice(0, DIFF_MAX_RENDER_ROWS),
    truncated: rows.length - DIFF_MAX_RENDER_ROWS,
  };
}
