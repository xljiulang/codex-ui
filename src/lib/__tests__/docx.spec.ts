import { describe, expect, it } from "vitest";
import { generateJSON } from "@tiptap/core";
import {
  Document,
  ExternalHyperlink,
  HeadingLevel,
  ImageRun,
  LevelFormat,
  AlignmentType,
  Packer,
  Paragraph,
  Table,
  TableCell,
  TableRow,
  TextRun,
} from "docx";
import {
  dataUriBytes,
  docxToHtml,
  imageSizeFromBytes,
  isDocxPath,
  jsonToDocx,
} from "../docx";
import { docxEditorExtensions } from "../docxEditor";

/** 1×1 透明 PNG */
const PNG_1X1_B64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

function b64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

function pngDataUri(): string {
  return `data:image/png;base64,${PNG_1X1_B64}`;
}

/** 用 docx 库生成一个含标题/加粗斜体/列表/表格/链接/图片的样例文档 */
async function fixtureDocxBytes(): Promise<ArrayBuffer> {
  const doc = new Document({
    numbering: {
      config: [
        {
          reference: "ordered-list",
          levels: [0, 1].map((level) => ({
            level,
            format: LevelFormat.DECIMAL,
            text: `%${level + 1}.`,
            alignment: AlignmentType.START,
            style: {
              paragraph: { indent: { left: 720 + level * 360, hanging: 360 } },
            },
          })),
        },
        {
          reference: "bullet-list",
          levels: [0, 1].map((level) => ({
            level,
            format: LevelFormat.BULLET,
            text: "•",
            alignment: AlignmentType.START,
            style: {
              paragraph: { indent: { left: 720 + level * 360, hanging: 360 } },
            },
          })),
        },
      ],
    },
    sections: [
      {
        children: [
          new Paragraph({
            heading: HeadingLevel.HEADING_1,
            children: [new TextRun({ text: "文档标题" })],
          }),
          new Paragraph({
            children: [
              new TextRun({ text: "普通 " }),
              new TextRun({ text: "加粗", bold: true }),
              new TextRun({ text: " 与 " }),
              new TextRun({ text: "斜体", italics: true }),
            ],
          }),
          new Paragraph({
            numbering: { reference: "ordered-list", level: 0 },
            children: [new TextRun({ text: "第一项" })],
          }),
          new Paragraph({
            numbering: { reference: "ordered-list", level: 0 },
            children: [new TextRun({ text: "第二项" })],
          }),
          new Paragraph({
            numbering: { reference: "bullet-list", level: 0 },
            children: [new TextRun({ text: "项目符号" })],
          }),
          new Table({
            rows: [
              new TableRow({
                children: [
                  new TableCell({
                    children: [
                      new Paragraph({ children: [new TextRun({ text: "A1" })] }),
                    ],
                  }),
                  new TableCell({
                    children: [
                      new Paragraph({ children: [new TextRun({ text: "B1" })] }),
                    ],
                  }),
                ],
              }),
            ],
          }),
          new Paragraph({
            children: [
              new ExternalHyperlink({
                link: "https://example.com",
                children: [new TextRun({ text: "示例链接" })],
              }),
            ],
          }),
          new Paragraph({
            children: [
              new ImageRun({
                type: "png",
                data: b64ToBytes(PNG_1X1_B64),
                transformation: { width: 100, height: 100 },
              }),
            ],
          }),
        ],
      },
    ],
  });
  const base64 = await Packer.toBase64String(doc);
  const bytes = b64ToBytes(base64);
  return bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  ) as ArrayBuffer;
}

describe("isDocxPath 扩展名判定", () => {
  it("大小写不敏感命中 .docx", () => {
    expect(isDocxPath("a.docx")).toBe(true);
    expect(isDocxPath("DIR/报告.DOCX")).toBe(true);
  });

  it("拒绝其它/无扩展名/点文件", () => {
    expect(isDocxPath("a.doc")).toBe(false);
    expect(isDocxPath("a.txt")).toBe(false);
    expect(isDocxPath(".docx")).toBe(false);
    expect(isDocxPath("docx")).toBe(false);
  });
});

describe("dataUriBytes / imageSizeFromBytes", () => {
  it("base64 data URI 解码与 PNG 尺寸解析", () => {
    const bytes = dataUriBytes(pngDataUri());
    expect(bytes).not.toBeNull();
    expect(imageSizeFromBytes(bytes!)).toEqual({ width: 1, height: 1 });
  });

  it("非 base64 / 损坏输入返回 null", () => {
    expect(dataUriBytes("https://x/y.png")).toBeNull();
    expect(dataUriBytes("data:image/png;base64,!!not-base64!!")).toBeNull();
    expect(imageSizeFromBytes(new Uint8Array([1, 2, 3]))).toBeNull();
  });
});

describe("docxToHtml 导入", () => {
  it("把样例 docx 转成可编辑 HTML（含图片尺寸属性）", async () => {
    const { html, warnings } = await docxToHtml(await fixtureDocxBytes());
    expect(html).toContain("<h1>");
    expect(html).toContain("<strong>加粗</strong>");
    expect(html).toContain("<em>斜体</em>");
    expect(html).toContain("<ol>");
    expect(html).toContain("<ul>");
    expect(html).toContain("<table>");
    expect(html).toContain('href="https://example.com"');
    expect(html).toContain("data:image/png;base64");
    expect(html).toContain('width="1"');
    expect(html).toContain('height="1"');
    expect(warnings).toEqual([]);
  });

  it("非 docx 输入抛错", async () => {
    const bogus = new ArrayBuffer(16);
    await expect(docxToHtml(bogus)).rejects.toThrow();
  });
});

describe("jsonToDocx 导出与往返", () => {
  it("HTML → 编辑器 JSON → docx → HTML 文本往返一致", async () => {
    const { html } = await docxToHtml(await fixtureDocxBytes());
    const json = generateJSON(html, docxEditorExtensions());
    const { base64, warnings } = await jsonToDocx(json);
    expect(base64.startsWith("UEs")).toBe(true); // ZIP 魔数
    expect(warnings).toEqual([]);

    const bytes = b64ToBytes(base64);
    const again = await docxToHtml(
      bytes.buffer.slice(
        bytes.byteOffset,
        bytes.byteOffset + bytes.byteLength,
      ) as ArrayBuffer,
    );
    expect(again.html).toContain("文档标题");
    expect(again.html).toContain("加粗");
    expect(again.html).toContain("斜体");
    expect(again.html).toContain("第一项");
    expect(again.html).toContain("第二项");
    expect(again.html).toContain("项目符号");
    expect(again.html).toContain("A1");
    expect(again.html).toContain("B1");
    expect(again.html).toContain("示例链接");
  });

  it("不支持的节点降级为纯文本段落", async () => {
    const json = {
      type: "doc",
      content: [
        {
          type: "taskList",
          content: [
            {
              type: "taskItem",
              content: [{ type: "paragraph", content: [{ type: "text", text: "任务一" }] }],
            },
          ],
        },
      ],
    };
    const { base64, warnings } = await jsonToDocx(json);
    expect(warnings).toEqual([]);
    const bytes = b64ToBytes(base64);
    const again = await docxToHtml(
      bytes.buffer.slice(
        bytes.byteOffset,
        bytes.byteOffset + bytes.byteLength,
      ) as ArrayBuffer,
    );
    expect(again.html).toContain("任务一");
  });

  it("无法导出的图片类型替换为占位文本并提示", async () => {
    const json = {
      type: "doc",
      content: [
        {
          type: "paragraph",
          content: [
            {
              type: "image",
              attrs: {
                src: "data:image/webp;base64,AAAA",
                alt: "",
              },
            },
          ],
        },
      ],
    };
    const { base64, warnings } = await jsonToDocx(json);
    expect(warnings.some((w) => w.includes("图片"))).toBe(true);
    const bytes = b64ToBytes(base64);
    const again = await docxToHtml(
      bytes.buffer.slice(
        bytes.byteOffset,
        bytes.byteOffset + bytes.byteLength,
      ) as ArrayBuffer,
    );
    expect(again.html).toContain("[图片]");
  });
});
