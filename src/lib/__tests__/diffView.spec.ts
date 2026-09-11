import { describe, expect, it } from "vitest";
import {
  DIFF_MAX_HIGHLIGHT_CHARS,
  DIFF_MAX_HIGHLIGHT_LINE_CHARS,
  DIFF_MAX_HIGHLIGHT_LINES,
  DIFF_MAX_RENDER_ROWS,
  canHighlightDiffRows,
  sliceDiffRows,
} from "../diffView";
import type { DiffRow } from "../types";

function ctxRows(count: number, text = "x"): DiffRow[] {
  return Array.from({ length: count }, (_, i) => ({
    kind: "ctx",
    oldNo: i + 1,
    newNo: i + 1,
    text,
  }));
}

describe("diffView 渲染预算", () => {
  it("sliceDiffRows 在上限内原样返回，超出则截断并给出省略行数", () => {
    const small = ctxRows(3);
    expect(sliceDiffRows(small)).toEqual({ visible: small, truncated: 0 });

    const many = ctxRows(DIFF_MAX_RENDER_ROWS + 5);
    const sliced = sliceDiffRows(many);
    expect(sliced.visible).toHaveLength(DIFF_MAX_RENDER_ROWS);
    expect(sliced.truncated).toBe(5);
  });

  it("canHighlightDiffRows：无语言或行数超上限时不高亮", () => {
    expect(canHighlightDiffRows(false, ctxRows(3))).toBe(false);
    expect(canHighlightDiffRows(true, ctxRows(DIFF_MAX_HIGHLIGHT_LINES + 1))).toBe(false);
    expect(canHighlightDiffRows(true, ctxRows(3))).toBe(true);
  });

  it("canHighlightDiffRows：单行超长（压缩成一行的大 JSON）时整份不高亮", () => {
    const rows: DiffRow[] = [
      { kind: "del", oldNo: 1, text: "a".repeat(DIFF_MAX_HIGHLIGHT_LINE_CHARS + 1) },
      { kind: "sep" },
      { kind: "add", newNo: 1, text: "b" },
    ];
    expect(canHighlightDiffRows(true, rows)).toBe(false);
  });

  it("canHighlightDiffRows：总字符量超预算时不高亮", () => {
    const perRow = DIFF_MAX_HIGHLIGHT_LINE_CHARS;
    const rows = ctxRows(Math.ceil(DIFF_MAX_HIGHLIGHT_CHARS / perRow) + 1, "a".repeat(perRow));
    expect(canHighlightDiffRows(true, rows)).toBe(false);
  });
});
