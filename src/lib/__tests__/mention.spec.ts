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
