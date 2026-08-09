import { describe, expect, it } from "vitest";
import {
  assemblePromptText,
  baseName,
  buildTurnInput,
  fileMentionSection,
  isImagePath,
  matchMentionToken,
  parseFileMentionSection,
  parseSkillMentionLinks,
  skillMentionLinks,
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

  it("assemblePromptText 顺序：文件段 + My request + 技能链接 + 用户输入", () => {
    expect(assemblePromptText("混合测试", mixedAttachments)).toBe(
      "\n# Files mentioned by the user:\n\n## a.cs: D:/repo/a.cs\n\n## b.txt: D:/repo/b.txt\n\n## My request:\n[$s1](C:/x/s1/SKILL.md) [$s2](C:/x/s2/SKILL.md) 混合测试\n",
    );
  });

  it("buildTurnInput 混合附件顺序：图片、文本、技能项", () => {
    const input = buildTurnInput("混合测试", mixedAttachments);
    expect(input).toEqual([
      { type: "localImage", path: "D:/repo/p.png" },
      {
        type: "text",
        text: "\n# Files mentioned by the user:\n\n## a.cs: D:/repo/a.cs\n\n## b.txt: D:/repo/b.txt\n\n## My request:\n[$s1](C:/x/s1/SKILL.md) [$s2](C:/x/s2/SKILL.md) 混合测试\n",
        text_elements: [],
      },
      { type: "skill", name: "s1", path: "C:/x/s1/SKILL.md" },
      { type: "skill", name: "s2", path: "C:/x/s2/SKILL.md" },
    ]);
  });

  it("混合文本可分别解析出文件引用与技能链接，stripMentionContext 还原用户输入", () => {
    const text = assemblePromptText("混合测试", [
      { type: "mention", name: "a.cs", path: "D:/repo/a.cs" },
      { type: "skill", name: "s1", path: "C:/x/s1/SKILL.md" },
    ]);
    expect(parseFileMentionSection(text)).toEqual([
      { name: "a.cs", path: "D:/repo/a.cs" },
    ]);
    expect(parseSkillMentionLinks(text)).toEqual([
      { name: "s1", path: "C:/x/s1/SKILL.md" },
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

  it("assemblePromptText 组装 VS Code 同款单条 text 输入", () => {
    const text = assemblePromptText("看一下这个文件", [
      { type: "mention", name: "a.cs", path: "D:/repo/a.cs" },
    ]);
    expect(text).toBe(
      "\n# Files mentioned by the user:\n\n## a.cs: D:/repo/a.cs\n\n## My request:\n看一下这个文件\n",
    );
    expect(assemblePromptText("hello", [])).toBe("hello\n");
  });

  it("skillMentionLinks 生成 VS Code 同款技能链接文本", () => {
    expect(
      skillMentionLinks([
        {
          type: "skill",
          name: "csharp-code-rules",
          path: "C:\\Users\\x\\.codex\\skills\\csharp-code-rules\\SKILL.md",
        },
      ]),
    ).toBe(
      "[$csharp-code-rules](C:/Users/x/.codex/skills/csharp-code-rules/SKILL.md) ",
    );
    expect(
      skillMentionLinks([
        { type: "skill", name: "a", path: "C:/x/SKILL.md" },
        { type: "skill", name: "b", path: "C:/y/SKILL.md" },
      ]),
    ).toBe("[$a](C:/x/SKILL.md) [$b](C:/y/SKILL.md) ");
    expect(skillMentionLinks([])).toBe("");
  });

  it("assemblePromptText 文件段 + My request + 技能链接 + 用户输入", () => {
    const text = assemblePromptText("逆向分析", [
      { type: "mention", name: "a.exe", path: "D:/repo/a.exe" },
      {
        type: "skill",
        name: "ida-pro-mcp:idapython",
        path: "C:\\Users\\x\\.codex\\plugins\\cache\\mrexodia\\ida-pro-mcp\\0.1.0\\skills\\idapython\\SKILL.md",
      },
    ]);
    expect(text).toBe(
      "\n# Files mentioned by the user:\n\n## a.exe: D:/repo/a.exe\n\n## My request:\n[$ida-pro-mcp:idapython](C:/Users/x/.codex/plugins/cache/mrexodia/ida-pro-mcp/0.1.0/skills/idapython/SKILL.md) 逆向分析\n",
    );
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
        text: "\n# Files mentioned by the user:\n\n## a.cs: D:/repo/a.cs\n\n## My request:\n[$csharp-code-rules](C:/Users/x/.codex/skills/csharp-code-rules/SKILL.md) 按规则检查\n",
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
    const text = assemblePromptText("看看这个", [
      { type: "mention", name: "a.cs", path: "D:/repo/a.cs" },
      { type: "mention", name: "b.cs", path: "D:/repo/b.cs" },
    ]);
    expect(parseFileMentionSection(text)).toEqual([
      { name: "a.cs", path: "D:/repo/a.cs" },
      { name: "b.cs", path: "D:/repo/b.cs" },
    ]);
    expect(stripMentionContext(text)).toBe("看看这个");
    expect(stripMentionContext("普通文本")).toBe("普通文本");
  });

  it("parseSkillMentionLinks / stripSkillLinks 用于界面回显", () => {
    const text = assemblePromptText("逆向分析", [
      {
        type: "skill",
        name: "ida-pro-mcp:idapython",
        path: "C:/Users/x/.codex/plugins/cache/mrexodia/ida-pro-mcp/0.1.0/skills/idapython/SKILL.md",
      },
    ]);
    expect(parseSkillMentionLinks(text)).toEqual([
      {
        name: "ida-pro-mcp:idapython",
        path: "C:/Users/x/.codex/plugins/cache/mrexodia/ida-pro-mcp/0.1.0/skills/idapython/SKILL.md",
      },
    ]);
    expect(stripSkillLinks("[$a](C:/x/SKILL.md) [$b](C:/y/SKILL.md) hello")).toBe(
      "hello",
    );
    expect(stripSkillLinks("普通文本")).toBe("普通文本");
    // 带文件段时 stripMentionContext 也去掉技能链接
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
