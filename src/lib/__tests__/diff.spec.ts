import { describe, expect, it } from "vitest";
import {
  applyReverseUnifiedDiff,
  buildInlineRows,
  parseUnifiedDiff,
} from "../diff";

const REPLACE_DIFF = [
  "@@ -1,3 +1,3 @@",
  " a",
  "-b",
  "+X",
  " c",
].join("\n");

describe("parseUnifiedDiff", () => {
  it("解析 hunk 头与行类型，忽略 diff 头与无换行标记", () => {
    const diff = [
      "diff --git a/x.txt b/x.txt",
      "--- a/x.txt",
      "+++ b/x.txt",
      "@@ -1,2 +1,2 @@",
      " a",
      "-b",
      "+B",
      "\\ No newline at end of file",
    ].join("\n");
    const hunks = parseUnifiedDiff(diff);
    expect(hunks).toHaveLength(1);
    expect(hunks[0]).toMatchObject({ oldStart: 1, oldCount: 2, newStart: 1, newCount: 2 });
    expect(hunks[0].lines).toEqual([
      { kind: "ctx", text: "a" },
      { kind: "del", text: "b" },
      { kind: "add", text: "B" },
    ]);
  });

  it("缺少行数时默认 1", () => {
    const hunks = parseUnifiedDiff("@@ -5 +7 @@\n x");
    expect(hunks[0]).toMatchObject({ oldStart: 5, oldCount: 1, newStart: 7, newCount: 1 });
  });
});

describe("applyReverseUnifiedDiff", () => {
  it("单 hunk 反向重建旧内容", () => {
    expect(applyReverseUnifiedDiff("a\nX\nc", REPLACE_DIFF)).toBe("a\nb\nc");
  });

  it("多 hunk 自底向上重建", () => {
    const oldLines = Array.from({ length: 10 }, (_, i) => `line${i + 1}`);
    const newLines = [...oldLines];
    newLines[2] = "changed3";
    newLines[7] = "changed8";
    const diff = [
      "@@ -3,1 +3,1 @@",
      "-line3",
      "+changed3",
      "@@ -8,1 +8,1 @@",
      "-line8",
      "+changed8",
    ].join("\n");
    expect(applyReverseUnifiedDiff(newLines.join("\n"), diff)).toBe(
      oldLines.join("\n"),
    );
  });

  it("新增文件反向得到空内容", () => {
    const diff = "@@ -0,0 +1,2 @@\n+n1\n+n2";
    expect(applyReverseUnifiedDiff("n1\nn2", diff)).toBe("");
  });

  it("删除文件由 diff 重建旧内容", () => {
    const diff = "@@ -1,3 +0,0 @@\n-a\n-b\n-c";
    expect(applyReverseUnifiedDiff("", diff)).toBe("a\nb\nc");
  });
});

describe("buildInlineRows", () => {
  it("变更处：旧行在上、分隔、新行在下，上下文配对", () => {
    const rows = buildInlineRows("a\nb\nc", "a\nX\nc", REPLACE_DIFF);
    expect(rows).toEqual([
      { kind: "ctx", oldNo: 1, newNo: 1, text: "a" },
      { kind: "del", oldNo: 2, text: "b" },
      { kind: "sep" },
      { kind: "add", newNo: 2, text: "X" },
      { kind: "ctx", oldNo: 3, newNo: 3, text: "c" },
    ]);
  });

  it("hunk 外未变行逐行配对，覆盖全部内容与行号", () => {
    const oldLines = Array.from({ length: 10 }, (_, i) => `line${i + 1}`);
    const newLines = [...oldLines];
    newLines[4] = "changed5";
    const diff = "@@ -5,1 +5,1 @@\n-line5\n+changed5";
    const rows = buildInlineRows(oldLines.join("\n"), newLines.join("\n"), diff);
    // 9 个 ctx + 1 del + 1 sep + 1 add = 12 行；覆盖全部旧/新内容
    expect(rows).toHaveLength(12);
    expect(rows[0]).toEqual({ kind: "ctx", oldNo: 1, newNo: 1, text: "line1" });
    expect(rows[4]).toEqual({ kind: "del", oldNo: 5, text: "line5" });
    expect(rows[5]).toEqual({ kind: "sep" });
    expect(rows[6]).toEqual({ kind: "add", newNo: 5, text: "changed5" });
    expect(rows[11]).toEqual({
      kind: "ctx",
      oldNo: 10,
      newNo: 10,
      text: "line10",
    });
  });

  it("纯新增不显示分隔，纯删除也不显示分隔", () => {
    const addRows = buildInlineRows("", "n1\nn2", "@@ -0,0 +1,2 @@\n+n1\n+n2");
    expect(addRows.some((r) => r.kind === "sep")).toBe(false);
    expect(addRows.filter((r) => r.kind === "add")).toHaveLength(2);

    const delRows = buildInlineRows("a\nb", "", "@@ -1,2 +0,0 @@\n-a\n-b");
    expect(delRows.some((r) => r.kind === "sep")).toBe(false);
    expect(delRows.filter((r) => r.kind === "del")).toHaveLength(2);
  });

  it("hunk 计数不一致时抛错（调用方回退统一 diff）", () => {
    const bad = "@@ -1,99 +1,1 @@\n+x";
    expect(() =>
      buildInlineRows("a", "x", bad),
    ).toThrow();
  });

  it("diff 尾部换行不产生多余行", () => {
    const rows = buildInlineRows("a\nb\nc", "a\nX\nc", REPLACE_DIFF + "\n");
    expect(rows).toHaveLength(5);
  });
});
