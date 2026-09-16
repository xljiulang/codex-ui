import { describe, expect, it } from "vitest";
import {
  dirNameOf,
  flattenResourceTree,
  formatFileTime,
  formatFileSize,
  isPathUnderRoot,
  joinFsPath,
  mergeSearchResults,
  type FsEntry,
  type RgHit,
} from "../sessionFs";

const root: FsEntry = {
  name: "repo",
  path: "D:\\repo",
  relPath: ".",
  isDir: true,
  size: null,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: 2,
};
const src: FsEntry = {
  name: "src",
  path: "D:\\repo\\src",
  relPath: "src",
  isDir: true,
  size: null,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: 1,
};
const main: FsEntry = {
  name: "main.ts",
  path: "D:\\repo\\src\\main.ts",
  relPath: "src/main.ts",
  isDir: false,
  size: 2048,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: null,
};
const readme: FsEntry = {
  name: "README.md",
  path: "D:\\repo\\README.md",
  relPath: "README.md",
  isDir: false,
  size: 512,
  modifiedAtMs: 0,
  createdAtMs: 0,
  childCount: null,
};

describe("formatFileSize", () => {
  it("格式化字节为人类可读", () => {
    expect(formatFileSize(null)).toBe("");
    expect(formatFileSize(0)).toBe("0 B");
    expect(formatFileSize(1023)).toBe("1023 B");
    expect(formatFileSize(1536)).toBe("1.5 KB");
    expect(formatFileSize(1024 * 1024)).toBe("1.0 MB");
    expect(formatFileSize(1024 * 1024 * 1024 * 2)).toBe("2.0 GB");
  });
});

describe("formatFileTime", () => {
  it("按相对时间格式化，缺失时间戳返回空串", () => {
    expect(formatFileTime(0)).toBe("");
    expect(formatFileTime(null)).toBe("");
    expect(formatFileTime(Date.now() - 5 * 60 * 1000)).toBe("5 分");
  });
});

describe("flattenResourceTree", () => {
  const children: Record<string, FsEntry[]> = {
    "D:\\repo": [src, readme],
    "D:\\repo\\src": [main],
  };

  it("根默认展开时只显示第一层，深层目录收起", () => {
    const rows = flattenResourceTree(root, children, new Set([root.path]));
    expect(rows).toHaveLength(3);
    expect(rows[0]).toMatchObject({ kind: "root", collapsed: false });
    expect(rows[1]).toMatchObject({ kind: "dir", depth: 1, collapsed: true });
    expect(rows[2]).toMatchObject({ kind: "file", depth: 1 });
  });

  it("展开子目录后递归展平，深度递增", () => {
    const rows = flattenResourceTree(
      root,
      children,
      new Set([root.path, src.path]),
    );
    expect(rows).toHaveLength(4);
    expect(rows[2]).toMatchObject({ kind: "file", depth: 2 });
  });

  it("根收起时只渲染根行", () => {
    const rows = flattenResourceTree(root, children, new Set());
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ kind: "root", collapsed: true });
  });

  it("未加载目录不产生子行（懒加载缓存为空）", () => {
    const rows = flattenResourceTree(root, { "D:\\repo": [src] }, new Set([root.path]));
    expect(rows).toHaveLength(2);
  });
});

describe("mergeSearchResults", () => {
  const contentA: FsEntry = {
    name: "z.ts",
    path: "D:\\repo\\z.ts",
    relPath: "z.ts",
    isDir: false,
    size: 1,
    modifiedAtMs: 0,
    createdAtMs: 0,
    childCount: null,
  };
  const contentB: FsEntry = {
    name: "b.ts",
    path: "D:\\repo\\src\\b.ts",
    relPath: "src/b.ts",
    isDir: false,
    size: 1,
    modifiedAtMs: 0,
    createdAtMs: 0,
    childCount: null,
  };

  it("文件名结果在前，内容-only 结果去重排序并附首个命中摘要", () => {
    const hits: RgHit[] = [
      { entry: main, lineNumber: 12, lineText: "const needle = 1;" },
      { entry: main, lineNumber: 99, lineText: "later duplicate" },
      { entry: contentA, lineNumber: 3, lineText: "z hit" },
      { entry: contentB, lineNumber: 4, lineText: "b hit" },
    ];
    const merged = mergeSearchResults([main, main, readme], hits);
    expect(merged.results.map((e) => e.relPath)).toEqual([
      "src/main.ts",
      "README.md",
      "src/b.ts",
      "z.ts",
    ]);
    expect(merged.snippets["d:\\repo\\src\\main.ts"]).toEqual({
      lineNumber: 12,
      text: "const needle = 1;",
    });
    expect(merged.snippets["d:\\repo\\src\\b.ts"]).toEqual({
      lineNumber: 4,
      text: "b hit",
    });
  });

  it("无内容命中时保持文件名结果原顺序", () => {
    const merged = mergeSearchResults([readme, main], []);
    expect(merged.results.map((e) => e.relPath)).toEqual([
      "README.md",
      "src/main.ts",
    ]);
    expect(merged.snippets).toEqual({});
  });
});

describe("joinFsPath", () => {
  it("拼接 Windows 路径并去尾分隔符", () => {
    expect(joinFsPath("D:\\repo", "src")).toBe("D:\\repo\\src");
    expect(joinFsPath("D:\\repo\\", "a.txt")).toBe("D:\\repo\\a.txt");
  });
});

describe("dirNameOf", () => {
  it("取最后一个分隔符前的部分并去尾分隔符", () => {
    expect(dirNameOf("D:\\repo\\src\\main.ts")).toBe("D:\\repo\\src");
    expect(dirNameOf("D:/repo/src/main.ts")).toBe("D:\\repo\\src");
  });

  it("盘符根返回带尾反斜杠，无分隔符返回空串", () => {
    expect(dirNameOf("D:\\")).toBe("D:\\");
    expect(dirNameOf("main.ts")).toBe("");
  });
});

describe("isPathUnderRoot", () => {
  it("边界判定：相等或位于根内为 true，外部/前缀歧义为 false", () => {
    expect(isPathUnderRoot("D:\\repo", "D:\\repo")).toBe(true);
    expect(isPathUnderRoot("D:\\repo", "D:\\repo\\src\\a.ts")).toBe(true);
    expect(isPathUnderRoot("D:\\repo", "D:\\repo2\\a.ts")).toBe(false);
    expect(isPathUnderRoot("D:\\repo", "D:\\other\\a.ts")).toBe(false);
    expect(isPathUnderRoot("", "D:\\repo")).toBe(false);
  });

  it("大小写不敏感且兼容正斜杠", () => {
    expect(isPathUnderRoot("d:\\repo", "D:\\REPO\\src\\a.ts")).toBe(true);
    expect(isPathUnderRoot("D:/repo", "D:/repo/src/a.ts")).toBe(true);
  });
});
