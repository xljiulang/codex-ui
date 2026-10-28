/** 单条工具输出在 UI 内存中的统一上限。 */
export const OUTPUT_TEXT_LINE_CAP = 5000;
export const OUTPUT_TEXT_BYTE_CAP = 512 * 1024;

export type TextTruncation = "none" | "lines" | "bytes" | "both";

export interface TextWindowLimits {
  maxLines: number;
  maxBytes?: number;
}

function utf8CodePointBytes(cp: number): number {
  if (cp <= 0x7f) return 1;
  if (cp <= 0x7ff) return 2;
  if (cp <= 0xffff) return 3;
  return 4;
}

function codePointStartBefore(text: string, end: number): number {
  if (end <= 0) return 0;
  const last = text.charCodeAt(end - 1);
  if (last >= 0xdc00 && last <= 0xdfff && end >= 2) {
    const prev = text.charCodeAt(end - 2);
    if (prev >= 0xd800 && prev <= 0xdbff) return end - 2;
  }
  return end - 1;
}

export function utf8ByteLength(text: string): number {
  let bytes = 0;
  for (let i = 0; i < text.length;) {
    const cp = text.codePointAt(i) ?? 0;
    bytes += utf8CodePointBytes(cp);
    i += cp > 0xffff ? 2 : 1;
  }
  return bytes;
}

export function tailStartByLines(text: string, maxLines: number): number {
  let remaining = Math.max(1, maxLines);
  for (let i = text.length - 1; i >= 0; i--) {
    if (text.charCodeAt(i) !== 10) continue;
    if (remaining === 1) return i + 1;
    remaining--;
  }
  return 0;
}

export function tailStartByBytes(text: string, maxBytes: number): number {
  if (maxBytes <= 0) return text.length;
  let bytes = 0;
  let i = text.length;
  while (i > 0) {
    const start = codePointStartBefore(text, i);
    const cp = text.codePointAt(start) ?? 0;
    const size = utf8CodePointBytes(cp);
    if (bytes + size > maxBytes) break;
    bytes += size;
    i = start;
  }
  return i;
}

export function capTextTail(
  text: string,
  limits: TextWindowLimits,
): { text: string; truncation: TextTruncation } {
  const lineStart = tailStartByLines(text, limits.maxLines);
  const byteStart =
    limits.maxBytes === undefined ? 0 : tailStartByBytes(text, limits.maxBytes);
  const start = Math.max(lineStart, byteStart);
  if (start === 0) return { text, truncation: "none" };
  const truncation =
    lineStart > 0 && byteStart > 0 ? "both" : lineStart > 0 ? "lines" : "bytes";
  return { text: text.slice(start), truncation };
}
