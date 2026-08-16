import { describe, expect, it } from "vitest";
import {
  baseName,
  buildTurnInput,
  fileMentionSection,
  isImagePath,
  matchMentionToken,
  parseFileMentionSection,
  parseInlineMentions,
  stripMentionContext,
  stripSkillLinks,
  toProtocolPath,
  toUserAttachment,
} from "../mention";
import type { UserInput } from "../types";

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

  it("词中 @ 不触发（前面必须有空格或行首）", () => {
    expect(matchMentionToken("看下@file")).toBeNull();
    expect(matchMentionToken("x@y")).toBeNull();
  });

  it("@ 后还有内容时不触发（触发词必须在文本末尾）", () => {
    expect(matchMentionToken("@file 更多文字")).toBeNull();
    expect(matchMentionToken("@file ")).toBeNull();
  });

  it("空格后 @ 触发并记录起始位置", () => {
    expect(matchMentionToken("看下 @file.txt")).toEqual({
      kind: "@",
      token: "file.txt",
      start: 3,
    });
  });

  it("文件名含空格：空格一输入菜单即关闭（token 必须是文本末尾）", () => {
    expect(matchMentionToken("@my file.txt")).toBeNull();
    expect(matchMentionToken("@my")).toEqual({
      kind: "@",
      token: "my",
      start: 0,
    });
  });

  it("$ 技能名允许冒号：插件技能全名可触发", () => {
    expect(matchMentionToken("$ida-pro-mcp:idapython")).toEqual({
      kind: "$",
      token: "ida-pro-mcp:idapython",
      start: 0,
    });
    expect(matchMentionToken("使用 $documents:documents")).toEqual({
      kind: "$",
      token: "documents:documents",
      start: 3,
    });
  });

  it("@ 文件引用不允许冒号（Windows 文件名不含冒号）", () => {
    expect(matchMentionToken("@a:b")).toBeNull();
    expect(matchMentionToken("$ida-pro-mcp")).toEqual({
      kind: "$",
      token: "ida-pro-mcp",
      start: 0,
    });
  });

  it("换行后 $ 触发", () => {
    expect(matchMentionToken("第一行\n$skill")).toEqual({
      kind: "$",
      token: "skill",
      start: 4,
    });
  });

  it("单独 $ 时 token 为空", () => {
    expect(matchMentionToken("$")).toEqual({ kind: "$", token: "", start: 0 });
  });
});

describe("混合使用 @ 文件引用与 $ 技能引用", () => {
  const mixedAttachments: UserInput[] = [
    { type: "mention", name: "a.cs", path: "D:/repo/a.cs" },
    { type: "mention", name: "b.txt", path: "D:/repo/b.txt" },
    { type: "skill", name: "s1", path: "C:/x/s1/SKILL.md" },
    { type: "skill", name: "s2", path: "C:/x/s2/SKILL.md" },
    { type: "localImage", path: "D:/repo/p.png" },
  ];

  it("buildTurnInput 混合附件顺序：图片、文本、技能项", () => {
    const input = buildTurnInput("混合测试", mixedAttachments);
    expect(input).toEqual([
      { type: "localImage", path: "D:/repo/p.png" },
      {
        type: "text",
        text: "混合测试\n",
        text_elements: [],
      },
      { type: "skill", name: "s1", path: "C:/x/s1/SKILL.md" },
      { type: "skill", name: "s2", path: "C:/x/s2/SKILL.md" },
    ]);
  });

  it("文件引用段可解析，stripMentionContext 还原用户输入", () => {
    const text =
      "\n# Files mentioned by the user:\n\n## a.cs: D:/repo/a.cs\n\n## My request:\n[$s1](C:/x/s1/SKILL.md) 混合测试\n";
    expect(parseFileMentionSection(text)).toEqual([
      { name: "a.cs", path: "D:/repo/a.cs" },
    ]);
    expect(stripMentionContext(text)).toBe("混合测试");
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
      path: "C:/x/a.cs",
    });
  });

  it("mention 路径统一转正斜杠，避免 Windows 反斜杠导致服务端读不到文件", () => {
    expect(toUserAttachment("a.cs", "D:\\repo\\src\\a.cs")).toEqual({
      type: "mention",
      name: "a.cs",
      path: "D:/repo/src/a.cs",
    });
    expect(toProtocolPath("D:\\repo\\src\\a.cs")).toBe("D:/repo/src/a.cs");
    expect(toProtocolPath("D:/repo/src/a.cs")).toBe("D:/repo/src/a.cs");
  });

  it("fileMentionSection 以 VS Code 扩展同款格式列出被引用文件", () => {
    expect(
      fileMentionSection([
        { type: "mention", name: "a.cs", path: "D:/repo/src/a.cs" },
      ]),
    ).toBe("\n# Files mentioned by the user:\n\n## a.cs: D:/repo/src/a.cs\n");
    expect(
      fileMentionSection([
        { type: "mention", name: "a.cs", path: "D:\\repo\\src\\a.cs" },
      ]),
    ).toBe("\n# Files mentioned by the user:\n\n## a.cs: D:/repo/src/a.cs\n");
    expect(
      fileMentionSection([
        { type: "mention", name: "a.cs", path: "D:/repo/a.cs" },
        { type: "mention", name: "b.cs", path: "D:/repo/b.cs" },
      ]),
    ).toBe(
      "\n# Files mentioned by the user:\n\n## a.cs: D:/repo/a.cs\n\n## b.cs: D:/repo/b.cs\n",
    );
    expect(fileMentionSection([{ type: "localImage", path: "D:/repo/a.png" }])).toBe(
      "",
    );
    expect(fileMentionSection([])).toBe("");
  });

  it("stripMentionContext 剥离 @ 与 $ 引用链接", () => {
    const text =
      "\n# Files mentioned by the user:\n\n## a.cs: D:/repo/a.cs\n\n## My request:\n[@documents](C:/x/plugins/documents) [$csharp-code-rules](C:/x/skills/csharp-code-rules/SKILL.md) 混合测试\n";
    expect(stripMentionContext(text)).toBe("混合测试");
    expect(
      stripMentionContext(
        "[@documents](C:/x/plugins/documents) 看下\n",
      ),
    ).toBe("看下");
    expect(
      stripMentionContext(
        "[@documents](C:/x/plugins/documents) [$csharp-code-rules](C:/x/SKILL.md) 看下\n",
      ),
    ).toBe("看下");
  });

  it("buildTurnInput 文件走单条 text、技能走结构化项且路径转正斜杠", () => {
    const input = buildTurnInput("按规则检查", [
      { type: "mention", name: "a.cs", path: "D:\\repo\\a.cs" },
      { type: "skill", name: "csharp-code-rules", path: "C:\\Users\\x\\.codex\\skills\\csharp-code-rules\\SKILL.md" },
      { type: "localImage", path: "D:\\repo\\a.png" },
    ]);
    expect(input).toEqual([
      { type: "localImage", path: "D:\\repo\\a.png" },
      {
        type: "text",
        text: "按规则检查\n",
        text_elements: [],
      },
      {
        type: "skill",
        name: "csharp-code-rules",
        path: "C:/Users/x/.codex/skills/csharp-code-rules/SKILL.md",
      },
    ]);
    expect(buildTurnInput("hi", [])).toEqual([
      { type: "text", text: "hi\n", text_elements: [] },
    ]);
  });

  it("parseInlineMentions 按顺序解析 [@name]/[$name] 链接与文本片段", () => {
    const text =
      "先 [@a.cs](D:/repo/a.cs) 中间 [$csharp-code-rules](C:/x/SKILL.md) 结尾";
    expect(parseInlineMentions(text)).toEqual([
      { type: "text", text: "先 " },
      { type: "ref", prefix: "@", name: "a.cs", path: "D:/repo/a.cs" },
      { type: "text", text: " 中间 " },
      {
        type: "ref",
        prefix: "$",
        name: "csharp-code-rules",
        path: "C:/x/SKILL.md",
      },
      { type: "text", text: " 结尾" },
    ]);
    expect(parseInlineMentions("纯文本")).toEqual([
      { type: "text", text: "纯文本" },
    ]);
    expect(parseInlineMentions("")).toEqual([]);
  });

  it("parseInlineMentions 无前缀本地路径链接识别为文件引用，URL/锚点保留为文本", () => {
    const text =
      "[a.cs](src/a.cs) 和 [lib-toml.json](.fingerprint/toml-1/lib-toml.json) 与 [百度](https://baidu.com) 还有 [节](#a)";
    expect(parseInlineMentions(text)).toEqual([
      { type: "ref", prefix: "@", name: "a.cs", path: "src/a.cs" },
      { type: "text", text: " 和 " },
      {
        type: "ref",
        prefix: "@",
        name: "lib-toml.json",
        path: ".fingerprint/toml-1/lib-toml.json",
      },
      { type: "text", text: " 与 [百度](https://baidu.com) 还有 [节](#a)" },
    ]);
    expect(
      parseInlineMentions(
        "[@documents](plugin://documents@openai-primary-runtime) 看下",
      ),
    ).toEqual([
      {
        type: "ref",
        prefix: "@",
        name: "documents",
        path: "plugin://documents@openai-primary-runtime",
      },
      { type: "text", text: " 看下" },
    ]);
  });

  it("stripMentionContext 剥离任意位置的内联引用链接", () => {
    expect(
      stripMentionContext(
        "先 [@a.cs](D:/repo/a.cs) 中间 [$csharp-code-rules](C:/x/SKILL.md) 结尾",
      ),
    ).toBe("先 中间 结尾");
    expect(
      stripMentionContext("[@documents](C:/x/documents) 看下"),
    ).toBe("看下");
    expect(stripMentionContext("先 [a.cs](src/a.cs) 看下")).toBe("先 看下");
  });

  it("buildTurnInput 结构化 skill 项不携带 source 标记", () => {
    const input = buildTurnInput("看下", [
      {
        type: "skill",
        name: "documents",
        path: "C:\\x\\plugins\\documents",
        source: "plugin",
      },
    ]);
    expect(input[input.length - 1]).toEqual({
      type: "skill",
      name: "documents",
      path: "C:/x/plugins/documents",
    });
  });

  it("buildTurnInput 保留图片附件作为 localImage 项", () => {
    expect(
      buildTurnInput("看这张图", [
        { type: "localImage", path: "D:\\repo\\截图.png" },
      ]),
    ).toEqual([
      { type: "localImage", path: "D:\\repo\\截图.png" },
      { type: "text", text: "看这张图\n", text_elements: [] },
    ]);
  });

  it("parseFileMentionSection / stripMentionContext 用于界面回显", () => {
    const text =
      "\n# Files mentioned by the user:\n\n## a.cs: D:/repo/a.cs\n\n## b.cs: D:/repo/b.cs\n\n## My request:\n看看这个\n";
    expect(parseFileMentionSection(text)).toEqual([
      { name: "a.cs", path: "D:/repo/a.cs" },
      { name: "b.cs", path: "D:/repo/b.cs" },
    ]);
    expect(stripMentionContext(text)).toBe("看看这个");
    expect(stripMentionContext("普通文本")).toBe("普通文本");
  });

  it("stripSkillLinks / stripMentionContext 剥离技能链接", () => {
    expect(stripSkillLinks("[$a](C:/x/SKILL.md) [$b](C:/y/SKILL.md) hello")).toBe(
      "hello",
    );
    expect(stripSkillLinks("普通文本")).toBe("普通文本");
    // 带文件段时 stripMentionContext 也去掉技能链接
    const text =
      "\n# Files mentioned by the user:\n\n## a.cs: D:/repo/a.cs\n\n## My request:\n[$ida-pro-mcp:idapython](C:/x/SKILL.md) 逆向分析\n";
    expect(stripMentionContext(text)).toBe("逆向分析");
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
