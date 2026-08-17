/** .docx 所见即所得编辑的导入/导出转换层 */

import type { JSONContent } from "@tiptap/core";
import type {
  ImageRun,
  Paragraph,
  ParagraphChild,
  Table,
} from "docx";

export const DOCX_EXT = "docx";

/** 大小写不敏感的 .docx 扩展名判定（点文件/无扩展名不命中） */
export function isDocxPath(name: string): boolean {
  const dot = name.lastIndexOf(".");
  return (
    dot > 0 &&
    dot < name.length - 1 &&
    name.slice(dot + 1).toLowerCase() === DOCX_EXT
  );
}

export interface DocxImportResult {
  html: string;
  warnings: string[];
}

export interface DocxExportResult {
  /** 可直接写入磁盘的 base64 内容 */
  base64: string;
  warnings: string[];
}

/**
 * docx 原始字节 → HTML（mammoth）：图片提取为 data URI，并在 img 上补
 * width/height 属性（按图片头解析，供导出与编辑器排版使用）。
 */
export async function docxToHtml(
  arrayBuffer: ArrayBuffer,
): Promise<DocxImportResult> {
  const mammoth = (await import("mammoth/mammoth.browser.js")).default;
  const result = await mammoth.convertToHtml(
    { arrayBuffer },
    {
      convertImage: mammoth.images.imgElement((image) =>
        image.read("base64").then((b64) => ({
          src: `data:${image.contentType};base64,${b64}`,
        })),
      ),
    },
  );
  return {
    html: attachImageSizes(result.value),
    warnings: result.messages.map((m) => m.message),
  };
}

/**
 * 编辑器 JSON → docx 字节（docx 库打包），返回 base64。
 * 支持的节点：段落/标题/列表/引用/代码块/分割线/表格/图片/链接与常见行内样式；
 * 不支持的节点降级为纯文本段落，无法导出的图片替换为占位文本。
 */
export async function jsonToDocx(
  json: JSONContent,
): Promise<DocxExportResult> {
  const {
    AlignmentType,
    Document,
    ExternalHyperlink,
    HeadingLevel,
    ImageRun,
    LevelFormat,
    Packer,
    Paragraph,
    ShadingType,
    Table,
    TableCell,
    TableRow,
    TextRun,
  } = await import("docx");
  const warnings: string[] = [];
  const listLevels = [0, 1, 2, 3, 4, 5, 6, 7, 8];
  const numberingConfig = [
    {
      reference: "ordered-list",
      levels: listLevels.map((level) => ({
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
      levels: listLevels.map((level) => ({
        level,
        format: LevelFormat.BULLET,
        text: level % 2 === 0 ? "•" : "◦",
        alignment: AlignmentType.START,
        style: {
          paragraph: { indent: { left: 720 + level * 360, hanging: 360 } },
        },
      })),
    },
  ];

  const runsForContent = (
    content: JSONContent[] | undefined,
  ): ParagraphChild[] => {
    const runs: ParagraphChild[] = [];
    for (const node of content ?? []) {
      if (node.type === "text" && node.text) {
        const opts = runOptions(node.marks);
        const link = node.marks?.find((m) => m.type === "link");
        const href = link?.attrs?.href;
        const run = new TextRun({ text: node.text, ...opts });
        if (typeof href === "string" && /^(https?:|mailto:)/i.test(href)) {
          runs.push(new ExternalHyperlink({ link: href, children: [run] }));
        } else {
          runs.push(run);
        }
      } else if (node.type === "hardBreak") {
        runs.push(new TextRun({ text: "", break: 1 }));
      } else if (node.type === "image") {
        const img = imageRunFromAttrs(node.attrs);
        if (img) {
          runs.push(img);
        } else {
          warnings.push("存在无法导出的图片，已替换为占位文本");
          runs.push(new TextRun({ text: "[图片]" }));
        }
      } else if (node.type === "text") {
        runs.push(new TextRun({ text: node.text ?? "" }));
      } else {
        runs.push(new TextRun({ text: textFromNode(node) }));
      }
    }
    return runs;
  };

  const pushBlocks = (
    nodes: JSONContent[] | undefined,
    depth: number,
    out: unknown[],
  ): void => {
    for (const node of nodes ?? []) {
      if (node.type === "paragraph") {
        out.push(
          new Paragraph({ children: runsForContent(node.content) }),
        );
      } else if (node.type === "heading") {
        const level = Math.min(6, Math.max(1, Number(node.attrs?.level) || 1));
        const heading =
          HeadingLevel[`HEADING_${level}` as keyof typeof HeadingLevel];
        out.push(
          new Paragraph({ heading, children: runsForContent(node.content) }),
        );
      } else if (node.type === "bulletList" || node.type === "orderedList") {
        const reference =
          node.type === "bulletList" ? "bullet-list" : "ordered-list";
        pushListItems(node.content, reference, depth, out);
      } else if (node.type === "blockquote") {
        for (const inner of node.content ?? []) {
          if (inner.type === "paragraph") {
            out.push(
              new Paragraph({
                indent: { left: 720 },
                children: runsForContent(inner.content),
              }),
            );
          } else {
            pushBlocks([inner], depth, out);
          }
        }
      } else if (node.type === "codeBlock") {
        const code = (node.content?.[0]?.text ?? "").replace(/\n$/, "");
        const lines = code === "" ? [""] : code.split("\n");
        for (const line of lines) {
          out.push(
            new Paragraph({
              shading: { type: ShadingType.CLEAR, fill: "F2F2F2" },
              children: [new TextRun({ text: line, font: { name: "Consolas" } })],
            }),
          );
        }
      } else if (node.type === "horizontalRule") {
        out.push(new Paragraph({ thematicBreak: true }));
      } else if (node.type === "table") {
        out.push(mapTable(node));
      } else if (node.type === "image") {
        const img = imageRunFromAttrs(node.attrs);
        if (!img) warnings.push("存在无法导出的图片，已替换为占位文本");
        out.push(
          new Paragraph({ children: [img ?? new TextRun({ text: "[图片]" })] }),
        );
      } else {
        // 不支持的节点：降级为纯文本段落
        out.push(
          new Paragraph({ children: [new TextRun({ text: textFromNode(node) })] }),
        );
      }
    }
  };

  const pushListItems = (
    items: JSONContent[] | undefined,
    reference: string,
    depth: number,
    out: unknown[],
  ): void => {
    for (const item of items ?? []) {
      const blocks = item.content ?? [];
      let sawParagraph = false;
      for (const block of blocks) {
        if (block.type === "paragraph") {
          sawParagraph = true;
          out.push(
            new Paragraph({
              numbering: { reference, level: Math.min(depth, 8) },
              children: runsForContent(block.content),
            }),
          );
        } else if (
          block.type === "bulletList" ||
          block.type === "orderedList"
        ) {
          const ref2 = block.type === "bulletList" ? "bullet-list" : "ordered-list";
          pushListItems(block.content, ref2, depth + 1, out);
        } else {
          pushBlocks([block], depth + 1, out);
        }
      }
      if (!sawParagraph) {
        out.push(
          new Paragraph({
            numbering: { reference, level: Math.min(depth, 8) },
            children: [],
          }),
        );
      }
    }
  };

  const mapTable = (node: JSONContent): Table => {
    const rows = (node.content ?? []).map((row) => {
      const cells = (row.content ?? []).map((cell) => {
        const cellChildren: (Paragraph | Table)[] = [];
        pushBlocks(cell.content, 0, cellChildren);
        return new TableCell({ children: cellChildren });
      });
      return new TableRow({ children: cells });
    });
    return new Table({ rows });
  };

  const imageRunFromAttrs = (
    attrs: JSONContent["attrs"],
  ): ImageRun | null => {
    const src = attrs?.src;
    if (typeof src !== "string" || !src.startsWith("data:")) return null;
    const bytes = dataUriBytes(src);
    const mime = src.slice(5, src.indexOf(";"));
    const type = IMAGE_MIME_TO_TYPE[mime as keyof typeof IMAGE_MIME_TO_TYPE];
    if (!bytes || !type) return null;
    const attrW = Number(attrs?.width);
    const attrH = Number(attrs?.height);
    const size =
      attrW > 0 && attrH > 0
        ? { width: attrW, height: attrH }
        : imageSizeFromBytes(bytes) ?? { width: 16, height: 16 };
    return new ImageRun({
      type,
      data: bytes,
      transformation: { width: size.width, height: size.height },
    });
  };

  const topChildren: (Paragraph | Table)[] = [];
  pushBlocks(json.content, 0, topChildren);
  const document = new Document({
    creator: "codex-ui",
    numbering: { config: numberingConfig },
    sections: [{ children: topChildren }],
  });
  const base64 = await Packer.toBase64String(document);
  return { base64, warnings };
}

const IMAGE_MIME_TO_TYPE = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/bmp": "bmp",
} as const;

/** 行内文本样式 → docx TextRun 选项 */
function runOptions(
  marks: JSONContent["marks"],
): Record<string, unknown> {
  const opts: Record<string, unknown> = {};
  for (const mark of marks ?? []) {
    if (mark.type === "bold") opts.bold = true;
    else if (mark.type === "italic") opts.italics = true;
    else if (mark.type === "underline") opts.underline = {};
    else if (mark.type === "strike") opts.strike = true;
    else if (mark.type === "code") opts.font = { name: "Consolas" };
  }
  return opts;
}

/** 递归提取节点纯文本（降级与未知节点共用） */
function textFromNode(node: JSONContent): string {
  if (typeof node.text === "string") return node.text;
  return (node.content ?? []).map(textFromNode).join("");
}

/** 把 data URI 解码为字节（仅 base64 形态） */
export function dataUriBytes(uri: string): Uint8Array | null {
  const comma = uri.indexOf(",");
  if (comma < 0) return null;
  const meta = uri.slice(0, comma);
  if (!meta.startsWith("data:") || !/;base64$/i.test(meta)) return null;
  try {
    const bin = atob(uri.slice(comma + 1));
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    return bytes;
  } catch {
    return null;
  }
}

/** 按图片文件头解析宽高（png/jpeg/gif/bmp），失败返回 null */
export function imageSizeFromBytes(
  bytes: Uint8Array,
): { width: number; height: number } | null {
  if (bytes.length < 24) return null;
  if (bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) {
    // PNG: IHDR 位于偏移 16，宽/高为大端 u32
    return {
      width: readU32BE(bytes, 16),
      height: readU32BE(bytes, 20),
    };
  }
  if (bytes[0] === 0x47 && bytes[1] === 0x49 && bytes[2] === 0x46) {
    // GIF: 宽/高为偏移 6 的小端 u16
    return {
      width: bytes[6] | (bytes[7] << 8),
      height: bytes[8] | (bytes[9] << 8),
    };
  }
  if (bytes[0] === 0x42 && bytes[1] === 0x4d) {
    // BMP: 宽/高为偏移 18/22 的小端 i32
    return {
      width: readU32LE(bytes, 18),
      height: Math.abs(readU32LE(bytes, 22)),
    };
  }
  if (bytes[0] === 0xff && bytes[1] === 0xd8) {
    return jpegSize(bytes);
  }
  return null;
}

function readU32BE(bytes: Uint8Array, at: number): number {
  return (
    ((bytes[at] << 24) |
      (bytes[at + 1] << 16) |
      (bytes[at + 2] << 8) |
      bytes[at + 3]) >>>
    0
  );
}

function readU32LE(bytes: Uint8Array, at: number): number {
  return (
    (bytes[at] |
      (bytes[at + 1] << 8) |
      (bytes[at + 2] << 16) |
      (bytes[at + 3] << 24)) >>>
    0
  );
}

/** JPEG：扫描段标记，SOF（C0-CF 且非 DHT/DAC）内携带宽高 */
function jpegSize(
  bytes: Uint8Array,
): { width: number; height: number } | null {
  let pos = 2;
  while (pos + 9 < bytes.length) {
    if (bytes[pos] !== 0xff) return null;
    const marker = bytes[pos + 1];
    if (marker === 0xd8 || (marker >= 0xd0 && marker <= 0xd7)) {
      pos += 2;
      continue;
    }
    const segLen = (bytes[pos + 2] << 8) | bytes[pos + 3];
    if (segLen < 2) return null;
    const isSof =
      (marker >= 0xc0 && marker <= 0xc3) ||
      (marker >= 0xc5 && marker <= 0xc7) ||
      (marker >= 0xc9 && marker <= 0xcb) ||
      (marker >= 0xcd && marker <= 0xcf);
    if (isSof) {
      return {
        height: (bytes[pos + 5] << 8) | bytes[pos + 6],
        width: (bytes[pos + 7] << 8) | bytes[pos + 8],
      };
    }
    pos += 2 + segLen;
  }
  return null;
}

/** 给 mammoth 输出 HTML 中的 img 补充 width/height 属性（像素） */
function attachImageSizes(html: string): string {
  if (typeof DOMParser === "undefined") return html;
  const parsed = new DOMParser().parseFromString(html, "text/html");
  for (const img of Array.from(parsed.body.querySelectorAll("img"))) {
    const src = img.getAttribute("src") ?? "";
    const bytes = dataUriBytes(src);
    const size = bytes ? imageSizeFromBytes(bytes) : null;
    if (!size) continue;
    if (!img.hasAttribute("width")) {
      img.setAttribute("width", String(size.width));
    }
    if (!img.hasAttribute("height")) {
      img.setAttribute("height", String(size.height));
    }
  }
  return parsed.body.innerHTML;
}
