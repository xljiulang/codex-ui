import type { UserInput } from "./types";

export interface MentionToken {
  kind: "@" | "$";
  token: string;
  start: number;
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
 * 解析输入框中光标处的 @ 触发词。
 * 先按「行首或空格 + 触发符 + 到行尾的非空白串」截取候选，再按类型校验字符集：
 * - @ 文件/插件引用：字母数字、下划线、点号、连字符与中文（不含冒号，Windows 文件名不含冒号）；
 * - $ 技能引用：额外允许冒号（插件技能名如 ida-pro-mcp:idapython 可整名触发）。
 * token 必须延伸到文本末尾，输入空格/非法字符后菜单随即关闭。
 */
export function matchMentionToken(text: string): MentionToken | null {
  const m = /(?:^|\s)([@$])(\S*)$/.exec(text);
  if (!m) return null;
  const kind = m[1] as "@" | "$";
  const token = m[2];
  const allowed =
    kind === "$"
      ? /^[\w.:\u4e00-\u9fff-]*$/
      : /^[\w.\u4e00-\u9fff-]*$/;
  if (!allowed.test(token)) return null;
  const triggerIdx =
    m.index + (m[0].startsWith("@") || m[0].startsWith("$") ? 0 : 1);
  return { kind, token, start: triggerIdx };
}

export function isImagePath(path: string): boolean {
  return /\.(png|jpe?g|gif|webp|bmp)$/i.test(path);
}

/**
 * 协议侧 mention 在 Windows 上用反斜杠路径时解析不到文件（实测只有
 * 正斜杠绝对路径能被读入上下文），统一转成正斜杠规避。
 */
export function toProtocolPath(path: string): string {
  return path.replace(/\\/g, "/");
}

/** 把文件/目录引用转成协议 UserInput（图片用 localImage，其余用 mention） */
export function toUserAttachment(name: string, path: string): UserInput {
  if (isImagePath(path)) return { type: "localImage", path };
  return { type: "mention", name, path: toProtocolPath(path) };
}

/**
 * 参考 VS Code Codex 扩展：普通聊天主链不发送独立结构化 mention 项，
 * 而是把文件引用序列化成文本段落，作为单条 text input 发送。
 */
const FILE_MENTION_HEADING = "# Files mentioned by the user:";
const MY_REQUEST_MARKER = "## My request:";

/** 文件引用段落：`# Files mentioned by the user:` + 每行 `## 名称: 路径` */
export function fileMentionSection(attachments: UserInput[]): string {
  const refs = attachments
    .filter((a) => a.type === "mention")
    .map((a) => `\n## ${a.name}: ${toProtocolPath(a.path)}\n`)
    .join("");
  return refs ? `\n${FILE_MENTION_HEADING}\n${refs}` : "";
}

/**
 * 技能引用文本：VS Code 扩展同款 Markdown 链接 `[$name](path)`，
 * 拼在 `## My request:` 之后、用户输入之前。
 */
export function skillMentionLinks(attachments: UserInput[]): string {
  const links = attachments
    .filter((a) => a.type === "skill")
    .map((a) => `[$${a.name}](${toProtocolPath(a.path)})`)
    .join(" ");
  return links ? `${links} ` : "";
}

/**
 * 组装单条 text 输入：文件引用段 + `## My request:` + 技能链接 + 用户输入。
 * 与 VS Code Codex 扩展抓包确认的格式一致。
 */
export function assemblePromptText(
  prompt: string,
  attachments: UserInput[],
): string {
  const section = fileMentionSection(attachments);
  const links = skillMentionLinks(attachments);
  return `${section ? `${section}\n${MY_REQUEST_MARKER}\n` : ""}${links}${prompt}\n`;
}

/** 组装一轮的协议输入：文件引用序列化进单条 text（VS Code 扩展同款），
 *  技能作为结构化 skill 项附带（fork 会把 SKILL.md 内容注入上下文），
 *  图片作为 localImage 项附带，路径统一转正斜杠。 */
export function buildTurnInput(
  prompt: string,
  attachments: UserInput[],
): UserInput[] {
  const text: UserInput = {
    type: "text",
    text: assemblePromptText(prompt, attachments),
    text_elements: [],
  };
  const images: UserInput[] = attachments.filter((a) => a.type === "localImage");
  const skills: UserInput[] = attachments
    .filter((a) => a.type === "skill")
    .map((a) => ({ type: "skill", name: a.name, path: toProtocolPath(a.path) }));
  return [...images, text, ...skills];
}

/** 从回显文本中解析被引用文件列表，供界面渲染引用标签 */
export function parseFileMentionSection(
  text: string,
): { name: string; path: string }[] {
  const headingIdx = text.indexOf(FILE_MENTION_HEADING);
  if (headingIdx < 0) return [];
  const rest = text.slice(headingIdx + FILE_MENTION_HEADING.length);
  const end = rest.indexOf(`\n${MY_REQUEST_MARKER}`);
  const body = end >= 0 ? rest.slice(0, end) : rest;
  const files: { name: string; path: string }[] = [];
  for (const line of body.split("\n")) {
    const m = /^##\s+(.+?):\s+(.+)$/.exec(line.trim());
    if (m) files.push({ name: m[1], path: m[2] });
  }
  return files;
}

/** 从文本中解析技能引用链接 `[$name](path)`，供界面渲染 `$name` 标签 */
export function parseSkillMentionLinks(
  text: string,
): { name: string; path: string }[] {
  const out: { name: string; path: string }[] = [];
  const re = /\[\$([^\]]+)\]\(([^)]+)\)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    out.push({ name: m[1], path: m[2] });
  }
  return out;
}

/** 移除文本开头的技能引用链接（assemblePromptText 自动生成的部分） */
export function stripSkillLinks(text: string): string {
  return text.replace(/^(?:\[\$[^\]]+\]\([^)]+\)\s*)+/, "");
}

/**
 * 去掉自动生成的文件引用段、`## My request:` 标记与技能链接，
 * 只保留用户实际输入。
 */
export function stripMentionContext(text: string): string {
  const marker = `\n${MY_REQUEST_MARKER}\n`;
  const idx = text.indexOf(marker);
  const request =
    idx >= 0 && text.slice(0, idx).includes(FILE_MENTION_HEADING)
      ? text.slice(idx + marker.length)
      : text;
  return stripSkillLinks(request).replace(/\n$/, "");
}

export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}
