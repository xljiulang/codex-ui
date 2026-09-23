import { describe, expect, it } from "vitest";
import type { ThreadItem, UserInput } from "../types";
import {
  enrichUserMessage,
  getUserMessageSummary,
  markdownToPlainText,
  summarizeUserMessage,
} from "../userMessage";

function textItem(text: string): UserInput {
  return { type: "text", text, text_elements: [] };
}

describe("summarizeUserMessage 用户消息派生摘要", () => {
  it("剥离 Files 引用段：text 保留原始 Markdown，navText 为纯文本", () => {
    const content = [
      textItem(
        [
          "# Files mentioned by the user:",
          "## a.cs: D:/a.cs",
          "",
          "## My request:",
          "请看看 **这个** 文件",
        ].join("\n"),
      ),
    ];
    expect(summarizeUserMessage(content)).toEqual({
      text: "请看看 **这个** 文件",
      navText: "请看看 这个 文件",
      isExecutePlan: false,
      executePlanText: "",
      planTitle: "",
    });
  });

  it("text 仅拼文本项；navText 按内容顺序附加图片/引用/技能占位", () => {
    const content = [
      { type: "localImage", path: "D:/a.png" },
      textItem("看图"),
      { type: "mention", name: "app.ts", path: "D:/app.ts" },
      { type: "skill", name: "skill-a", path: "D:/SKILL.md" },
    ] as UserInput[];
    const summary = summarizeUserMessage(content);
    // 与原气泡 userText（map bodyText + join("\n")）行为一致
    expect(summary.text).toBe("\n看图\n\n");
    expect(summary.navText).toBe("[图片] 看图 @app.ts $skill-a");
    expect(summary.isExecutePlan).toBe(false);
  });

  it("仅 Files 段无正文时：navText 降级为 @文件名（多文件空格连接）", () => {
    const single = summarizeUserMessage([
      textItem(
        [
          "# Files mentioned by the user:",
          "## a.cs: D:/a.cs",
          "",
          "## My request:",
          "",
        ].join("\n"),
      ),
    ]);
    expect(single.text).toBe("");
    expect(single.navText).toBe("@a.cs");

    const multi = summarizeUserMessage([
      textItem(
        [
          "# Files mentioned by the user:",
          "## a.cs: D:/a.cs",
          "## b.ts: D:/b.ts",
          "",
          "## My request:",
          "",
        ].join("\n"),
      ),
    ]);
    expect(multi.navText).toBe("@a.cs @b.ts");
  });

  it("有正文时 navText 不并入文件名；Files 段解析不到名称时仍为空", () => {
    const withBody = summarizeUserMessage([
      textItem(
        [
          "# Files mentioned by the user:",
          "## a.cs: D:/a.cs",
          "",
          "## My request:",
          "请看看这个",
          "",
        ].join("\n"),
      ),
    ]);
    expect(withBody.navText).toBe("请看看这个");

    const unparsed = summarizeUserMessage([
      textItem(
        [
          "# Files mentioned by the user:",
          "## 无冒号的坏行",
          "",
          "## My request:",
          "",
        ].join("\n"),
      ),
    ]);
    expect(unparsed.navText).toBe("");
  });

  it("执行计划消息：忽略大小写识别前缀，planTitle 取首个标题", () => {
    const summary = summarizeUserMessage([
      textItem("please implement this plan:\n# 重构方案\n- 步骤1\n- 步骤2"),
    ]);
    expect(summary.isExecutePlan).toBe(true);
    expect(summary.executePlanText).toBe("# 重构方案\n- 步骤1\n- 步骤2");
    expect(summary.planTitle).toBe("重构方案");
    expect(summary.navText).toContain("重构方案");
  });

  it("执行计划无标题行时 planTitle 回退「计划」", () => {
    const summary = summarizeUserMessage([
      textItem("PLEASE IMPLEMENT THIS PLAN:\n- 步骤A"),
    ]);
    expect(summary.isExecutePlan).toBe(true);
    expect(summary.executePlanText).toBe("- 步骤A");
    expect(summary.planTitle).toBe("计划");
  });

  it("空/缺失 content 与协议外形状：摘要字段全为空，不抛错", () => {
    for (const content of [undefined, null, []]) {
      expect(summarizeUserMessage(content)).toEqual({
        text: "",
        navText: "",
        isExecutePlan: false,
        executePlanText: "",
        planTitle: "",
      });
    }
    expect(
      summarizeUserMessage([
        { type: "inputText", text: "hi" },
      ] as unknown as UserInput[]),
    ).toMatchObject({ text: "", navText: "" });
  });
});

describe("getUserMessageSummary / enrichUserMessage", () => {
  const item = (content?: UserInput[]): ThreadItem =>
    ({
      id: "u1",
      type: "userMessage",
      ...(content === undefined ? {} : { content }),
    }) as ThreadItem;

  it("有 derived 时直接返回持久化摘要", () => {
    const derived = {
      text: "旧文本",
      navText: "旧导航",
      isExecutePlan: true,
      executePlanText: "# 旧计划",
      planTitle: "旧计划",
    };
    const withDerived = {
      id: "u1",
      type: "userMessage",
      content: [],
      derived,
    } as ThreadItem;
    expect(getUserMessageSummary(withDerived)).toBe(derived);
  });

  it("缺省 derived 时即时回退计算，与持久化结果一致", () => {
    const content = [textItem("请看看 **这个** 文件")];
    const raw = item(content);
    expect(getUserMessageSummary(raw)).toEqual(summarizeUserMessage(content));
    expect(
      getUserMessageSummary({ id: "a1", type: "agentMessage" } as ThreadItem),
    ).toMatchObject({ text: "", navText: "" });
  });

  it("enrichUserMessage 为 userMessage 写 derived，非用户消息不动", () => {
    const raw = item([textItem("PLEASE IMPLEMENT THIS PLAN:\n# 标题")]);
    expect(enrichUserMessage(raw)).toBe(raw);
    expect(raw.derived).toMatchObject({
      isExecutePlan: true,
      planTitle: "标题",
    });

    const agent = { id: "a1", type: "agentMessage" } as ThreadItem;
    expect(enrichUserMessage(agent)).toBe(agent);
    expect("derived" in agent).toBe(false);
  });
});

describe("markdownToPlainText marked 同引擎提取", () => {
  it("Markdown 标题/列表/代码块/链接转为纯文本（既有行为不变）", () => {
    expect(
      markdownToPlainText(
        [
          "# 标题",
          "- 列表项 **加粗**",
          "`inline` [链接](https://x)  ![图](a.png)",
          "```ts\nconst x = 1;\n```",
          "结尾",
        ].join("\n"),
      ),
    ).toBe("标题 列表项 加粗 inline 链接 图 结尾");
  });

  it("token 语义：标题/列表/任务/有序列表/引用/表格/行内代码/加粗/链接/图片", () => {
    const md = [
      "# 标题",
      "",
      "- 项目A",
      "- [x] 已完成",
      "",
      "1. 第一",
      "",
      "> 引用 **强调**",
      "",
      "| 列A | 列B |",
      "| --- | --- |",
      "| 1 | 2 |",
      "",
      "行内 `code` 与 **粗体** [链接](https://x) ![图](a.png)",
    ].join("\n");
    expect(markdownToPlainText(md)).toBe(
      "标题 项目A 已完成 第一 引用 强调 列A 列B 1 2 行内 code 与 粗体 链接 图",
    );
  });

  it("fenced/缩进代码块不进入预览；br 换行与多段空白折叠为单空格", () => {
    const fenced = [
      "前言",
      "",
      "```ts",
      "const x = 1;",
      "```",
      "",
      "第一行",
      "第二行",
    ].join("\n");
    expect(markdownToPlainText(fenced)).toBe("前言 第一行 第二行");

    const indented = ["正文", "", "    缩进代码行", "", "结尾"].join("\n");
    expect(markdownToPlainText(indented)).toBe("正文 结尾");

    expect(markdownToPlainText("前置 <br> 后置")).toBe("前置 <br> 后置");
  });

  it("html token 按字面文本参与（与气泡渲染一致）；空/纯代码输入输出为空串", () => {
    expect(markdownToPlainText("<span>可见文字</span> 后缀")).toBe(
      "<span>可见文字</span> 后缀",
    );
    expect(markdownToPlainText("a<b>c")).toBe("a<b>c");
    expect(markdownToPlainText("<user_instructions>")).toBe(
      "<user_instructions>",
    );
    expect(markdownToPlainText("把 <ABCD> 里的东西改掉")).toBe(
      "把 <ABCD> 里的东西改掉",
    );
    expect(markdownToPlainText("")).toBe("");
    expect(markdownToPlainText("```\ncode\n```")).toBe("");
  });
});
