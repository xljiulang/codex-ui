import { describe, expect, it } from "vitest";
import {
  UTF8_BOM,
  buildSaveContent,
  detectEol,
  normalizeForEditor,
  stripBom,
} from "../editorFile";

describe("detectEol", () => {
  it("LF 文件", () => {
    expect(detectEol("a\nb\nc")).toBe("\n");
  });
  it("CRLF 文件", () => {
    expect(detectEol("a\r\nb\r\nc")).toBe("\r\n");
  });
  it("混合行尾：CRLF 占多数按 CRLF，LF 占多数按 LF", () => {
    expect(detectEol("a\r\nb\r\nc\nd")).toBe("\r\n");
    expect(detectEol("a\r\nb\nc\nd")).toBe("\n");
  });
  it("空文件/无换行按 LF", () => {
    expect(detectEol("")).toBe("\n");
    expect(detectEol("abc")).toBe("\n");
  });
});

describe("stripBom", () => {
  it("有 BOM 时剥离并标记", () => {
    expect(stripBom(`${UTF8_BOM}abc`)).toEqual({ text: "abc", hadBom: true });
  });
  it("无 BOM 原样返回", () => {
    expect(stripBom("abc")).toEqual({ text: "abc", hadBom: false });
  });
});

describe("normalizeForEditor", () => {
  it("CRLF 统一为 LF，LF 不变", () => {
    expect(normalizeForEditor("a\r\nb\r\n", "\r\n")).toBe("a\nb\n");
    expect(normalizeForEditor("a\nb\n", "\n")).toBe("a\nb\n");
  });
});

describe("buildSaveContent", () => {
  it("LF 文档 + CRLF 原文件 → 还原 CRLF", () => {
    expect(buildSaveContent("a\nb\n", "\r\n", false)).toBe("a\r\nb\r\n");
  });
  it("LF 文档 + LF 原文件 → 保持 LF", () => {
    expect(buildSaveContent("a\nb\n", "\n", false)).toBe("a\nb\n");
  });
  it("还原 BOM", () => {
    expect(buildSaveContent("a\n", "\n", true)).toBe(`${UTF8_BOM}a\n`);
  });
  it("文档内残留 CRLF 先归一为 LF 再还原，避免双重 CRLF", () => {
    expect(buildSaveContent("a\r\nb\n", "\r\n", false)).toBe("a\r\nb\r\n");
  });
});
