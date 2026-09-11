import { describe, expect, it } from "vitest";
import {
  DIFF_AUTO_BRIEF_ROWS,
  diffKindLabel,
  gitStatusLetter,
  normalizeDiffKind,
  shouldBriefDiffRows,
} from "../gitChanges";

describe("gitChanges 状态映射", () => {
  it("gitStatusLetter 使用 A/M/D/R/U/C", () => {
    expect(gitStatusLetter("added")).toBe("A");
    expect(gitStatusLetter("modified")).toBe("M");
    expect(gitStatusLetter("deleted")).toBe("D");
    expect(gitStatusLetter("renamed")).toBe("R");
    expect(gitStatusLetter("untracked")).toBe("U");
    expect(gitStatusLetter("conflicted")).toBe("C");
  });

  it("normalizeDiffKind 兼容 GitFileStatus 与协议 kind", () => {
    // GitFileStatus
    expect(normalizeDiffKind("added")).toBe("add");
    expect(normalizeDiffKind("untracked")).toBe("add");
    expect(normalizeDiffKind("deleted")).toBe("delete");
    expect(normalizeDiffKind("modified")).toBe("modify");
    expect(normalizeDiffKind("renamed")).toBe("modify");
    expect(normalizeDiffKind("conflicted")).toBe("modify");
    // 协议字符串
    expect(normalizeDiffKind("add")).toBe("add");
    expect(normalizeDiffKind("delete")).toBe("delete");
    // 对象形态
    expect(normalizeDiffKind({ type: "add" })).toBe("add");
    expect(normalizeDiffKind({ type: "delete" })).toBe("delete");
    // 兜底
    expect(normalizeDiffKind("update")).toBe("modify");
    expect(normalizeDiffKind(undefined)).toBe("modify");
    expect(normalizeDiffKind(null)).toBe("modify");
  });

  it("diffKindLabel 输出新增/删除/修改", () => {
    expect(diffKindLabel("add")).toBe("新增");
    expect(diffKindLabel("delete")).toBe("删除");
    expect(diffKindLabel("modify")).toBe("修改");
  });

  it("diff 行数超过阈值时默认进简要显示", () => {
    expect(shouldBriefDiffRows(0)).toBe(false);
    expect(shouldBriefDiffRows(DIFF_AUTO_BRIEF_ROWS)).toBe(false);
    expect(shouldBriefDiffRows(DIFF_AUTO_BRIEF_ROWS + 1)).toBe(true);
  });
});
