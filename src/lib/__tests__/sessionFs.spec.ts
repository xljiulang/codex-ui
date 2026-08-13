import { describe, expect, it } from "vitest";
import {
  flattenResourceTree,
  formatFileSize,
  formatFileTime,
  isTextFile,
  joinFsPath,
  type FsEntry,
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
  it("0 与缺失返回空串", () => {
    expect(formatFileTime(0)).toBe("");
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

describe("joinFsPath", () => {
  it("拼接 Windows 路径并去尾分隔符", () => {
    expect(joinFsPath("D:\\repo", "src")).toBe("D:\\repo\\src");
    expect(joinFsPath("D:\\repo\\", "a.txt")).toBe("D:\\repo\\a.txt");
  });
});

describe("isTextFile", () => {
  it("文本/代码扩展名与常见文件名命中", () => {
    expect(isTextFile("a.txt")).toBe(true);
    expect(isTextFile("README.md")).toBe(true);
    expect(isTextFile("main.ts")).toBe(true);
    expect(isTextFile("Dockerfile")).toBe(true);
    expect(isTextFile(".gitignore")).toBe(true);
    expect(isTextFile("Makefile")).toBe(true);
    expect(isTextFile("config.yaml")).toBe(true);
    expect(isTextFile("LICENSE")).toBe(true);
  });

  it("二进制/媒体/未知无扩展名文件不命中", () => {
    expect(isTextFile("pic.png")).toBe(false);
    expect(isTextFile("a.zip")).toBe(false);
    expect(isTextFile("app.exe")).toBe(false);
    expect(isTextFile("notes")).toBe(false);
    expect(isTextFile("")).toBe(false);
  });

  it("大小写不敏感", () => {
    expect(isTextFile("A.TXT")).toBe(true);
    expect(isTextFile("Main.TS")).toBe(true);
    expect(isTextFile("DOCKERFILE")).toBe(true);
  });
});
