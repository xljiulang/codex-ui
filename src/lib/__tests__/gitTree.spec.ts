import { describe, expect, it } from "vitest";
import type { GitFile } from "../gitChanges";
import { buildGitTree, flattenRows } from "../gitTree";

function file(
  path: string,
  overrides: Partial<GitFile> = {},
): GitFile {
  return {
    path,
    status: "modified",
    staged: false,
    worktree: true,
    ...overrides,
  };
}

describe("buildGitTree", () => {
  it("按路径构建目录树，目录与文件均按名称排序", () => {
    const files = [
      file("z-root.txt"),
      file("b/x.txt"),
      file("a/deep/y.md"),
      file("a/deep/x.md"),
      file("a/top.rs"),
    ];
    const tree = buildGitTree(files, new Set());

    // 根级：目录 a、b 在前（A-Z），文件 z-root.txt 在后
    expect(tree.map((n) => n.name)).toEqual(["a", "b", "z-root.txt"]);
    const dirA = tree[0];
    expect(dirA.kind).toBe("dir");
    if (dirA.kind === "dir") {
      expect(dirA.depth).toBe(0);
      expect(dirA.relPath).toBe("a");
      expect(dirA.children.map((c) => c.name)).toEqual(["deep", "top.rs"]);
      const deep = dirA.children[0];
      expect(deep.kind).toBe("dir");
      if (deep.kind === "dir") {
        expect(deep.depth).toBe(1);
        expect(deep.relPath).toBe("a/deep");
        expect(deep.children.map((c) => c.name)).toEqual(["x.md", "y.md"]);
      }
    }
  });

  it("目录聚合子级计数与暂存/未暂存/未跟踪标记", () => {
    const files = [
      file("src/a.ts", { staged: true }),
      file("src/b.ts", { status: "untracked" }),
      file("src/c.ts", { staged: true, worktree: false }),
    ];
    const tree = buildGitTree(files, new Set());
    const dir = tree[0];
    expect(dir.kind).toBe("dir");
    if (dir.kind === "dir") {
      expect(dir.childCount).toBe(3);
      expect(dir.hasStaged).toBe(true);
      expect(dir.hasUnstaged).toBe(true); // b.ts 未暂存（untracked）
      expect(dir.hasUntracked).toBe(true);
    }
  });

  it("无文件返回空树", () => {
    expect(buildGitTree([], new Set())).toEqual([]);
  });
});

describe("flattenRows", () => {
  it("展开目录时输出深度优先可见行，折叠目录跳过子级", () => {
    const files = [
      file("src/a.ts"),
      file("src/deep/b.ts"),
      file("src/deep/c.ts"),
      file("top.md"),
    ];
    const tree = buildGitTree(files, new Set());
    const flat = flattenRows(tree);
    expect(flat.map((n) => n.relPath)).toEqual([
      "src",
      "src/deep",
      "src/deep/b.ts",
      "src/deep/c.ts",
      "src/a.ts",
      "top.md",
    ]);

    const collapsed = buildGitTree(files, new Set(["src"]));
    const flatCollapsed = flattenRows(collapsed);
    expect(flatCollapsed.map((n) => n.relPath)).toEqual(["src", "top.md"]);
  });
});
