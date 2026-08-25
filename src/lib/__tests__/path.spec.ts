import { describe, expect, it } from "vitest";
import {
  dirNameOf,
  isPathUnderRoot,
  joinFsPath,
  normalizeFsPath,
  normalizePathKey,
  pathEquals,
  relPathOf,
  stripWindowsVerbatim,
} from "../path";

describe("stripWindowsVerbatim", () => {
  it("剥离 \\?\\ 与 \\?\\UNC\\ 前缀，其余原样保留", () => {
    expect(stripWindowsVerbatim("\\\\?\\C:\\a\\b")).toBe("C:\\a\\b");
    expect(stripWindowsVerbatim("\\\\?\\UNC\\srv\\share\\f.txt")).toBe(
      "\\\\srv\\share\\f.txt",
    );
    expect(stripWindowsVerbatim("C:\\a\\b")).toBe("C:\\a\\b");
    expect(stripWindowsVerbatim("C:/x/bundled")).toBe("C:/x/bundled");
    expect(stripWindowsVerbatim("https://example.com/repo.git")).toBe(
      "https://example.com/repo.git",
    );
  });
});

describe("normalizeFsPath / normalizePathKey", () => {
  it("正斜杠统一为反斜杠，去尾分隔符，盘符根保留尾分隔符", () => {
    expect(normalizeFsPath("D:/repo")).toBe("D:\\repo");
    expect(normalizeFsPath("D:\\repo\\")).toBe("D:\\repo");
    expect(normalizeFsPath("D:\\")).toBe("D:\\");
    expect(normalizeFsPath("D:/")).toBe("D:\\");
    expect(normalizeFsPath("\\\\?\\C:\\a\\b")).toBe("C:\\a\\b");
  });

  it("比较/键形态：反斜杠 + 小写 + 去尾（含盘符根）", () => {
    expect(normalizePathKey("D:/Repo\\src\\")).toBe("d:\\repo\\src");
    expect(normalizePathKey("D:\\")).toBe("d:");
    expect(normalizePathKey("\\\\?\\C:\\A\\B")).toBe("c:\\a\\b");
  });
});

describe("pathEquals", () => {
  it("正反斜杠与大小写等价", () => {
    expect(pathEquals("D:/repo/src/a.ts", "D:\\repo\\src\\a.ts")).toBe(true);
    expect(pathEquals("d:/repo", "D:\\REPO")).toBe(true);
    expect(pathEquals("D:\\repo", "D:\\repo2")).toBe(false);
    expect(pathEquals("D:", "D:\\")).toBe(true);
  });
});

describe("isPathUnderRoot", () => {
  it("混合分隔符与大小写均正确判定", () => {
    expect(isPathUnderRoot("D:/repo", "D:\\repo\\src\\a.ts")).toBe(true);
    expect(isPathUnderRoot("d:\\repo", "D:/REPO")).toBe(true);
    expect(isPathUnderRoot("D:\\", "D:/repo/a.ts")).toBe(true);
    expect(isPathUnderRoot("D:\\repo", "D:\\repo2\\a.ts")).toBe(false);
    expect(isPathUnderRoot("", "D:\\repo")).toBe(false);
  });
});

describe("joinFsPath", () => {
  it("base 正斜杠/尾分隔符均归一为反斜杠拼接", () => {
    expect(joinFsPath("D:/repo", "src")).toBe("D:\\repo\\src");
    expect(joinFsPath("D:\\repo\\", "a.txt")).toBe("D:\\repo\\a.txt");
    expect(joinFsPath("D:\\", "a.txt")).toBe("D:\\a.txt");
  });
});

describe("dirNameOf", () => {
  it("混合分隔符返回反斜杠 dirname，盘符根保留尾分隔符", () => {
    expect(dirNameOf("D:/repo/src/main.ts")).toBe("D:\\repo\\src");
    expect(dirNameOf("D:\\")).toBe("D:\\");
    expect(dirNameOf("D:\\a.txt")).toBe("D:\\");
    expect(dirNameOf("main.ts")).toBe("");
  });
});

describe("relPathOf", () => {
  it("混合分隔符/大小写剥离根前缀，非根下原样返回", () => {
    expect(relPathOf("D:/repo", "D:\\repo\\src\\a.ts")).toBe("src\\a.ts");
    expect(relPathOf("D:\\repo", "D:/REPO/src/a.ts")).toBe("src\\a.ts");
    expect(relPathOf("D:\\repo", "D:\\repo")).toBe("");
    expect(relPathOf("D:\\repo", "D:\\other\\a.ts")).toBe("D:\\other\\a.ts");
    expect(relPathOf("D:\\repo", "src/a.ts")).toBe("src/a.ts");
    expect(relPathOf("D:\\", "D:\\a.txt")).toBe("a.txt");
  });
});
