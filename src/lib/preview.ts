/** 特殊文件类型预览：扩展名识别与字节解码（PDF / 图像 / 视频 / 音频 / 表格 / DOCX / PPTX） */

export type PreviewType =
  | "pdf"
  | "image"
  | "video"
  | "audio"
  | "xlsx"
  | "docx"
  | "pptx";

const PDF_EXT = "pdf";
const DOCX_EXT = "docx";
const PPTX_EXT = "pptx";
/** 表格类扩展名：统一走只读表格预览（SheetJS 解析，见 lib/xlsx.ts） */
const SHEET_EXTS = ["xlsx", "xlsm", "xlsb", "xls", "ods", "csv", "tsv"];
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
const VIDEO_EXTS = [
  "mp4",
  "webm",
  "mkv",
  "mov",
  "avi",
  "m4v",
  "ogv",
  "mpg",
  "mpeg",
  "wmv",
  "flv",
  "3gp",
];
const AUDIO_EXTS = [
  "mp3",
  "wav",
  "ogg",
  "oga",
  "flac",
  "m4a",
  "aac",
  "opus",
  "wma",
  "mid",
  "midi",
  "aiff",
  "ape",
];

/** 取文件名小写扩展名（含点）；无扩展名/点文件返回 null */
export function extOf(name: string): string | null {
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return null;
  return name.slice(dot + 1).toLowerCase();
}

/**
 * 按扩展名判定预览类型：.pdf → "pdf"；常见图像格式 → "image"；常见视频/音频格式 →
 * "video"/"audio"（asset 协议流式播放）；表格类（xlsx/xlsm/xlsb/xls/ods/csv/tsv）→ "xlsx"；
 * .docx → "docx"（docx-preview 排版预览）；.pptx → "pptx"（pptx-preview 版式预览）；
 * 其余返回 null（走原有文本探测/编辑器逻辑）。大小写不敏感。
 */
export function previewTypeForName(name: string): PreviewType | null {
  const ext = extOf(name);
  if (!ext) return null;
  if (ext === PDF_EXT) return "pdf";
  if (SHEET_EXTS.includes(ext)) return "xlsx";
  if (ext === DOCX_EXT) return "docx";
  if (ext === PPTX_EXT) return "pptx";
  if (IMAGE_EXTS.includes(ext)) return "image";
  if (VIDEO_EXTS.includes(ext)) return "video";
  if (AUDIO_EXTS.includes(ext)) return "audio";
  return null;
}
