import { describe, expect, it } from "vitest";
import {
  baseName,
  isImagePath,
  matchMentionToken,
  toUserAttachment,
} from "../mention";

describe("matchMentionToken", () => {
  it("匹配行首 @ 及带扩展名/下划线的文件名", () => {
    expect(matchMentionToken("@useCodex.ts")).toEqual({
      kind: "@",
      token: "useCodex.ts",
      start: 0,
    });
  });

  it("匹配中文文件名", () => {
    expect(matchMentionToken("看下 @计划文档.md")).toEqual({
      kind: "@",
      token: "计划文档.md",
      start: 3,
    });
  });

  it("匹配 $ 技能触发词", () => {
    expect(matchMentionToken("$skill-x")).toEqual({
      kind: "$",
      token: "skill-x",
      start: 0,
    });
  });

  it("普通文本不匹配", () => {
    expect(matchMentionToken("这是一段普通文本")).toBeNull();
    expect(matchMentionToken("")).toBeNull();
  });

  it("单独 @ 时 token 为空", () => {
    expect(matchMentionToken("@")).toEqual({ kind: "@", token: "", start: 0 });
  });
});

describe("toUserAttachment / isImagePath / baseName", () => {
  it("图片转 localImage，其余转 mention", () => {
    expect(toUserAttachment("a.png", "C:\\x\\a.png")).toEqual({
      type: "localImage",
      path: "C:\\x\\a.png",
    });
    expect(toUserAttachment("a.cs", "C:\\x\\a.cs")).toEqual({
      type: "mention",
      name: "a.cs",
      path: "C:\\x\\a.cs",
    });
  });

  it("isImagePath 识别常见图片扩展名", () => {
    expect(isImagePath("x.PNG")).toBe(true);
    expect(isImagePath("x.webp")).toBe(true);
    expect(isImagePath("x.txt")).toBe(false);
  });

  it("baseName 兼容两种分隔符", () => {
    expect(baseName("D:\\a\\b\\c.md")).toBe("c.md");
    expect(baseName("/a/b/c.md")).toBe("c.md");
  });
});
