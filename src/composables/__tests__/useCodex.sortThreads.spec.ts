import { describe, expect, it } from "vitest";
import { sortThreads } from "../useCodex";
import type { ThreadSummary } from "../../lib/types";

function t(id: string, recency: number, pinned = false): ThreadSummary {
  return {
    id,
    preview: id,
    createdAt: recency,
    recencyAt: recency,
    isPinned: pinned,
  };
}

describe("历史会话排序", () => {
  it("固定会话置顶，其余按最近时间降序", () => {
    const list = [
      t("a", 100),
      t("pinned2", 10, true),
      t("b", 200),
      t("pinned1", 50, true),
    ];
    const sorted = sortThreads(list).map((x) => x.id);
    // 固定项之间也按最近时间降序
    expect(sorted).toEqual(["pinned1", "pinned2", "b", "a"]);
  });

  it("recencyAt 缺失时回退 updatedAt/createdAt", () => {
    const list = [
      { id: "x", preview: "x", createdAt: 10, updatedAt: 30 },
      { id: "y", preview: "y", createdAt: 20, updatedAt: 40 },
    ] as ThreadSummary[];
    expect(sortThreads(list).map((x) => x.id)).toEqual(["y", "x"]);
  });
});
