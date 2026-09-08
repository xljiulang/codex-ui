import { describe, expect, it } from "vitest";
import { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import {
  Reference,
  docToRuns,
  runsToText,
  runsToWireText,
  tokenStartPos,
} from "../richEditor";
import type { UserInput } from "../types";

function createEditor(content: unknown) {
  const el = document.createElement("div");
  return new Editor({
    element: el,
    extensions: [StarterKit, Reference],
    content: {
      type: "doc",
      content: content as never,
    },
  });
}

const refJSON = (id: string, kind: string, label: string) => ({
  type: "reference",
  attrs: { refId: id, kind, label },
});

describe("docToRuns / runsToText", () => {
  it("单段落文本与引用按顺序生成 runs，引用不贡献文本", () => {
    const doc = {
      type: "doc",
      content: [
        {
          type: "paragraph",
          content: [
            { type: "text", text: "看 " },
            refJSON("r1", "file", "a.cs"),
            { type: "text", text: "然后" },
            refJSON("r2", "skill", "csharp-code-rules"),
          ],
        },
      ],
    };
    expect(docToRuns(doc)).toEqual([
      { kind: "text", text: "看 " },
      { kind: "ref", refId: "r1", refKind: "file", label: "a.cs" },
      { kind: "text", text: "然后" },
      { kind: "ref", refId: "r2", refKind: "skill", label: "csharp-code-rules" },
    ]);
    expect(runsToText(docToRuns(doc))).toBe("看 然后");
  });

  it("多段落文本以 \\n 连接", () => {
    const doc = {
      type: "doc",
      content: [
        { type: "paragraph", content: [{ type: "text", text: "第一行" }] },
        {
          type: "paragraph",
          content: [
            refJSON("r1", "plugin", "documents"),
            { type: "text", text: "第二行" },
          ],
        },
      ],
    };
    const runs = docToRuns(doc);
    expect(runs).toEqual([
      { kind: "text", text: "第一行\n" },
      { kind: "ref", refId: "r1", refKind: "plugin", label: "documents" },
      { kind: "text", text: "第二行" },
    ]);
    expect(runsToText(runs)).toBe("第一行\n第二行");
  });

  it("嵌套列表（bulletList/listItem）文本按顺序提取，避免发送按钮误禁用", () => {
    const doc = {
      type: "doc",
      content: [
        {
          type: "bulletList",
          content: [
            {
              type: "listItem",
              content: [
                { type: "paragraph", content: [{ type: "text", text: "第一项" }] },
              ],
            },
            {
              type: "listItem",
              content: [
                { type: "paragraph", content: [{ type: "text", text: "第二项" }] },
              ],
            },
          ],
        },
      ],
    };
    const runs = docToRuns(doc);
    expect(runsToText(runs).trim().length).toBeGreaterThan(0);
    expect(runsToText(runs)).toBe("第一项\n第二项");
  });

  it("块引用/代码块内文本可提取", () => {
    const doc = {
      type: "doc",
      content: [
        {
          type: "blockquote",
          content: [
            { type: "paragraph", content: [{ type: "text", text: "引用行" }] },
          ],
        },
        {
          type: "codeBlock",
          content: [{ type: "text", text: "code 内容" }],
        },
      ],
    };
    expect(runsToText(docToRuns(doc))).toBe("引用行\ncode 内容");
  });

  it("列表项内嵌引用 chip 仍按顺序产出 ref run", () => {
    const doc = {
      type: "doc",
      content: [
        {
          type: "bulletList",
          content: [
            {
              type: "listItem",
              content: [
                {
                  type: "paragraph",
                  content: [
                    { type: "text", text: "用 " },
                    refJSON("r1", "file", "a.cs"),
                  ],
                },
              ],
            },
          ],
        },
      ],
    };
    const runs = docToRuns(doc);
    expect(runsToText(runs)).toBe("用 ");
    expect(runs[1]).toEqual({
      kind: "ref",
      refId: "r1",
      refKind: "file",
      label: "a.cs",
    });
  });

  it("真实编辑器创建 bulletList 后 runsToText 非空（复现输入 `- 内容`）", () => {
    const editor = createEditor([
      {
        type: "bulletList",
        content: [
          {
            type: "listItem",
            content: [
              { type: "paragraph", content: [{ type: "text", text: "内容" }] },
            ],
          },
        ],
      },
    ]);
    const runs = docToRuns(editor.getJSON());
    expect(runsToText(runs).trim().length).toBeGreaterThan(0);
    expect(editor.getText()).toContain("内容");
    editor.destroy();
  });
});

describe("runsToWireText 内联链接序列化", () => {
  it("纯内联：插件/技能链接内联在文本原位，文件节点忽略", () => {
    const refs = new Map<string, UserInput>([
      ["r1", { type: "mention", name: "a.cs", path: "D:/repo/src/a.cs" }],
      [
        "r2",
        {
          type: "skill",
          name: "documents",
          path: "C:/x/documents",
          source: "plugin",
          pluginId: "documents@openai-primary-runtime",
        },
      ],
      ["r3", { type: "skill", name: "csharp-code-rules", path: "C:/x/SKILL.md", source: "skill" }],
    ]);
    const runs = [
      { kind: "text", text: "先 " },
      { kind: "ref", refId: "r1", refKind: "file", label: "a.cs" },
      { kind: "text", text: "中间 " },
      { kind: "ref", refId: "r2", refKind: "plugin", label: "documents" },
      { kind: "text", text: "再 " },
      { kind: "ref", refId: "r3", refKind: "skill", label: "csharp-code-rules" },
      { kind: "text", text: "结尾" },
    ] as const;
    expect(runsToWireText(runs as never, refs)).toBe(
      "先 中间 [@documents](plugin://documents@openai-primary-runtime) 再 [$csharp-code-rules](C:/x/SKILL.md) 结尾",
    );
  });

  it("文件节点不参与内联文本", () => {
    const refs = new Map<string, UserInput>([
      ["r1", { type: "mention", name: "a.txt", path: "D:/repo/a.txt" }],
      ["r2", { type: "mention", name: "b.txt", path: "D:/repo/b.txt" }],
    ]);
    const runs = [
      { kind: "ref", refId: "r1", refKind: "file", label: "a.txt" },
      { kind: "text", text: "看下" },
      { kind: "ref", refId: "r2", refKind: "file", label: "b.txt" },
    ] as const;
    expect(runsToWireText(runs as never, refs)).toBe(
      "看下",
    );
    expect(runsToWireText([{ kind: "text", text: "纯文本" }] as never, new Map())).toBe(
      "纯文本",
    );
  });

  it("跨段落以 \\n 连接，缺失 refId 的引用跳过", () => {
    const refs = new Map<string, UserInput>([
      ["r1", { type: "mention", name: "a.cs", path: "D:/repo/a.cs" }],
    ]);
    const runs = [
      { kind: "text", text: "第一行" },
      { kind: "text", text: "\n" },
      { kind: "ref", refId: "r1", refKind: "file", label: "a.cs" },
      { kind: "ref", refId: "missing", refKind: "skill", label: "x" },
      { kind: "text", text: "第二行" },
    ] as const;
    expect(runsToWireText(runs as never, refs)).toBe(
      "第一行\n第二行",
    );
  });
});

describe("tokenStartPos（真实 Tiptap Editor 坐标）", () => {
  it("单段落 token 删除后保留前置文本", () => {
    const editor = createEditor([
      {
        type: "paragraph",
        content: [{ type: "text", text: "看 @doc" }],
      },
    ]);
    const end = editor.state.doc.content.size;
    editor.commands.setTextSelection(end);
    const caret = editor.state.selection.from;
    const from = tokenStartPos(editor.state.doc, caret, 3);
    editor.chain().deleteRange({ from, to: caret }).run();
    expect(editor.getText()).toBe("看 ");
    editor.destroy();
  });

  it("chip 前置时 token 删除仍只移除触发词", () => {
    const editor = createEditor([
      {
        type: "paragraph",
        content: [
          { type: "text", text: "a " },
          refJSON("r1", "file", "x.txt"),
          { type: "text", text: "@doc" },
        ],
      },
    ]);
    const end = editor.state.doc.content.size;
    editor.commands.setTextSelection(end);
    const caret = editor.state.selection.from;
    const from = tokenStartPos(editor.state.doc, caret, 3);
    editor.chain().deleteRange({ from, to: caret }).run();
    expect(editor.getText()).toBe("a ");
    expect(editor.getJSON()).toEqual({
      type: "doc",
      content: [
        {
          type: "paragraph",
          content: [
            { type: "text", text: "a " },
            {
              type: "reference",
              attrs: { refId: "r1", kind: "file", label: "x.txt" },
            },
          ],
        },
      ],
    });
    editor.destroy();
  });

  it("token 在新段落开头：删除不影响上一段落", () => {
    const editor = createEditor([
      { type: "paragraph", content: [{ type: "text", text: "第一行" }] },
      { type: "paragraph", content: [{ type: "text", text: "@doc" }] },
    ]);
    const end = editor.state.doc.content.size;
    editor.commands.setTextSelection(end);
    const caret = editor.state.selection.from;
    const from = tokenStartPos(editor.state.doc, caret, 3);
    editor.chain().deleteRange({ from, to: caret }).run();
    const json = editor.getJSON() as {
      content: { type: string; content?: { type: string; text?: string }[] }[];
    };
    expect(editor.getText()).not.toContain("@doc");
    expect(json.content[0].content?.[0]?.text).toBe("第一行");
    expect(json.content[1].content?.length ?? 0).toBe(0);
    editor.destroy();
  });
});
