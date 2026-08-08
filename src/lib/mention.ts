import type { UserInput } from "./types";

export interface MentionToken {
  kind: "@" | "$";
  token: string;
  start: number;
}

/** 最近引用记录（仅内存） */
export interface RecentRef {
  name: string;
  path: string;
}

/** fuzzyFileSearch 协议返回的文件/目录条目 */
export interface FuzzyFileResult {
  root: string;
  path: string;
  match_type: "file" | "directory";
  file_name: string;
  score: number;
  indices: number[] | null;
}

/**
 * 解析输入框中光标处的 @ / $ 触发词。
 * token 允许字母数字、下划线、点号、连字符与中文，避免文件名（含扩展名/中文）被截断。
 */
export function matchMentionToken(text: string): MentionToken | null {
  const m = /(?:^|\s)([@$])([\w.\u4e00-\u9fff-]*)$/.exec(text);
  if (!m) return null;
  const triggerIdx =
    m.index + (m[0].startsWith("@") || m[0].startsWith("$") ? 0 : 1);
  return { kind: m[1] as "@" | "$", token: m[2], start: triggerIdx };
}

export function isImagePath(path: string): boolean {
  return /\.(png|jpe?g|gif|webp|bmp)$/i.test(path);
}

/** 把文件/目录引用转成协议 UserInput（图片用 localImage，其余用 mention） */
export function toUserAttachment(name: string, path: string): UserInput {
  if (isImagePath(path)) return { type: "localImage", path };
  return { type: "mention", name, path };
}

export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

/** 最近引用：按 path 去重、最新在前、上限 max */
export function pushRecent(list: RecentRef[], item: RecentRef, max = 10): RecentRef[] {
  const rest = list.filter((r) => r.path !== item.path);
  return [item, ...rest].slice(0, max);
}
