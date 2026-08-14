import { describe, expect, it } from "vitest";
import { gitDiffKind, gitStatusLabel, gitStatusLetter } from "../gitChanges";

describe("gitChanges 状态映射", () => {
  it("gitStatusLabel 覆盖全部状态", () => {
    expect(gitStatusLabel("added")).toBe("新增");
    expect(gitStatusLabel("modified")).toBe("修改");
    expect(gitStatusLabel("deleted")).toBe("删除");
    expect(gitStatusLabel("renamed")).toBe("重命名");
    expect(gitStatusLabel("untracked")).toBe("未跟踪");
    expect(gitStatusLabel("conflicted")).toBe("冲突");
  });

  it("gitStatusLetter 使用 A/M/D/R/U/C", () => {
    expect(gitStatusLetter("added")).toBe("A");
    expect(gitStatusLetter("modified")).toBe("M");
    expect(gitStatusLetter("deleted")).toBe("D");
    expect(gitStatusLetter("renamed")).toBe("R");
    expect(gitStatusLetter("untracked")).toBe("U");
    expect(gitStatusLetter("conflicted")).toBe("C");
  });

  it("gitDiffKind 映射 diff 窗口 kind", () => {
    expect(gitDiffKind("added")).toBe("add");
    expect(gitDiffKind("untracked")).toBe("add");
    expect(gitDiffKind("deleted")).toBe("delete");
    expect(gitDiffKind("modified")).toBe("modify");
    expect(gitDiffKind("renamed")).toBe("modify");
    expect(gitDiffKind("conflicted")).toBe("modify");
  });
});
