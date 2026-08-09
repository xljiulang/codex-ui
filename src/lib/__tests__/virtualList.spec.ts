import { describe, expect, it } from "vitest";
import {
  buildPrefixHeights,
  computeVisibleRange,
  estimateRowHeight,
  rangeIndices,
} from "../virtualList";

describe("estimateRowHeight 按类型估计", () => {
  it("各类型返回默认高度", () => {
    expect(estimateRowHeight({ kind: "sep" })).toBe(34);
    expect(estimateRowHeight({ kind: "msg", item: { type: "userMessage" } })).toBe(60);
    expect(estimateRowHeight({ kind: "msg", item: { type: "agentMessage" } })).toBe(180);
    expect(estimateRowHeight({ kind: "msg", item: { type: "plan" } })).toBe(180);
    expect(estimateRowHeight({ kind: "msg", item: { type: "reasoning" } })).toBe(80);
    expect(estimateRowHeight({ kind: "msg", item: { type: "imageView" } })).toBe(340);
    expect(estimateRowHeight({ kind: "msg", item: { type: "commandExecution" } })).toBe(110);
    expect(estimateRowHeight({ kind: "msg", item: { type: "unknown" } })).toBe(60);
  });
});

describe("buildPrefixHeights", () => {
  it("累加各行高度", () => {
    const rows = [{ kind: "sep" }, { kind: "msg", item: { type: "userMessage" } }];
    const prefix = buildPrefixHeights(rows, (r) => estimateRowHeight(r));
    expect(prefix).toEqual([34, 94]);
  });
});

describe("computeVisibleRange", () => {
  const prefix = Array.from({ length: 1000 }, (_, i) => (i + 1) * 100);

  it("空列表与不可测尺寸返回安全值", () => {
    expect(computeVisibleRange([], 0, 500)).toEqual({ start: 0, end: 0 });
    expect(computeVisibleRange(prefix, 0, 0)).toEqual({ start: 0, end: 999 });
  });

  it("顶部：视口 500px 覆盖前 5 行 + overscan", () => {
    const r = computeVisibleRange(prefix, 0, 500, 5);
    expect(r).toEqual({ start: 0, end: 9 });
  });

  it("中间滚动位置二分正确", () => {
    const r = computeVisibleRange(prefix, 500, 500, 5);
    // 可见 6..10 行（index 5..9），overscan 前后各 5
    expect(r).toEqual({ start: 0, end: 14 });
  });

  it("滚动超过总高时落在末尾", () => {
    const r = computeVisibleRange(prefix, 200_000, 500, 5);
    expect(r).toEqual({ start: 994, end: 999 });
  });

  it("rangeIndices 含两端", () => {
    expect(rangeIndices(2, 5)).toEqual([2, 3, 4, 5]);
    expect(rangeIndices(0, 0)).toEqual([0]);
  });
});
