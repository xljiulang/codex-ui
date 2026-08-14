/** 特殊文件类型预览：扩展名识别与字节解码（PDF / 图像） */

export type PreviewType = "pdf" | "image";

const PDF_EXT = "pdf";
const IMAGE_EXTS = [
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "bmp",
  "svg",
  "ico",
  "avif",
];

/** 取文件名小写扩展名（含点）；无扩展名/点文件返回 null */
export function extOf(name: string): string | null {
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return null;
  return name.slice(dot + 1).toLowerCase();
}

/**
 * 按扩展名判定预览类型：.pdf → "pdf"；常见图像格式 → "image"；
 * 其余返回 null（走原有文本探测/编辑器逻辑）。大小写不敏感。
 */
export function previewTypeForName(name: string): PreviewType | null {
  const ext = extOf(name);
  if (!ext) return null;
  if (ext === PDF_EXT) return "pdf";
  if (IMAGE_EXTS.includes(ext)) return "image";
  return null;
}

/** base64 字符串 → Uint8Array（PDF 预览字节数据源） */
export function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) {
    bytes[i] = bin.charCodeAt(i);
  }
  return bytes;
}
