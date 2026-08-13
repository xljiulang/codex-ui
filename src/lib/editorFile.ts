/** 编辑器文件格式处理：行尾（EOL）与 BOM 的检测/还原 */

export type EditorEol = "\n" | "\r\n";

export const UTF8_BOM = "\uFEFF";

/**
 * 检测主导行尾：CRLF 数量 ≥ 独立 LF 且存在 CRLF 时按 CRLF，
 * 否则按 LF（空文件/无换行文件按 LF）。
 */
export function detectEol(content: string): EditorEol {
  let crlf = 0;
  let lf = 0;
  for (let i = 0; i < content.length; i++) {
    if (content[i] !== "\n") continue;
    if (i > 0 && content[i - 1] === "\r") crlf++;
    else lf++;
  }
  return crlf > 0 && crlf >= lf ? "\r\n" : "\n";
}

/** 剥离 UTF-8 BOM，返回正文与是否存在 BOM */
export function stripBom(content: string): { text: string; hadBom: boolean } {
  if (content.startsWith(UTF8_BOM)) {
    return { text: content.slice(UTF8_BOM.length), hadBom: true };
  }
  return { text: content, hadBom: false };
}

/** 供编辑器使用的规范化文本：CRLF 统一为 LF（编辑器内固定 LF） */
export function normalizeForEditor(content: string, eol: EditorEol): string {
  return eol === "\r\n" ? content.replace(/\r\n/g, "\n") : content;
}

/**
 * 编辑器文档 → 保存负载：先把文档内可能残留的 CRLF 归一为 LF，
 * 再按原文件主导行尾还原，最后还原 BOM。
 */
export function buildSaveContent(
  doc: string,
  eol: EditorEol,
  hadBom: boolean,
): string {
  let out = doc.replace(/\r\n/g, "\n");
  if (eol === "\r\n") out = out.replace(/\n/g, "\r\n");
  if (hadBom) out = UTF8_BOM + out;
  return out;
}
