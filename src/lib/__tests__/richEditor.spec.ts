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
});

describe("runsToWireText 内联链接序列化", () => {
  it("文件输出相对路径、插件输出 plugin:// URI、技能保持绝对路径，顺序交织", () => {
    const refs = new Map<string, UserInput>([
      ["r1", { type: "mention", name: "a.cs", path: "D:\\repo\\src\\a.cs" }],
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
    expect(runsToWireText(runs as never, refs, "D:/repo")).toBe(
      "先 [a.cs](src/a.cs) 中间 [@documents](plugin://documents@openai-primary-runtime) 再 [$csharp-code-rules](C:/x/SKILL.md) 结尾",
    );
  });

  it("文件不在工作目录下时回退绝对路径", () => {
    const refs = new Map<string, UserInput>([
      ["r1", { type: "mention", name: "x.txt", path: "C:/other/x.txt" }],
    ]);
    const runs = [{ kind: "ref", refId: "r1", refKind: "file", label: "x.txt" }] as const;
    expect(runsToWireText(runs as never, refs, "D:/repo")).toBe(
      "[x.txt](C:/other/x.txt) ",
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
      "第一行\n[a.cs](D:/repo/a.cs) 第二行",
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
