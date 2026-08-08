import { describe, expect, it } from "vitest";
import {
  baseName,
  isImagePath,
  matchMentionToken,
  pushRecent,
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

describe("pushRecent", () => {
  it("最新在前并按路径去重", () => {
    const list = pushRecent([], { name: "a", path: "/a" });
    const list2 = pushRecent(list, { name: "b", path: "/b" });
    const list3 = pushRecent(list2, { name: "a2", path: "/a" });
    expect(list3.map((r) => r.path)).toEqual(["/a", "/b"]);
    expect(list3[0].name).toBe("a2");
  });

  it("超过上限时截断", () => {
    let list: { name: string; path: string }[] = [];
    for (let i = 0; i < 12; i++) {
      list = pushRecent(list, { name: `f${i}`, path: `/f${i}` });
    }
    expect(list).toHaveLength(10);
    expect(list[0].path).toBe("/f11");
    expect(list[9].path).toBe("/f2");
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
