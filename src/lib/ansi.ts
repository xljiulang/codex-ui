/**
 * 终端 ANSI 转义序列 -> HTML。
 * 支持常见 SGR（颜色/加粗/斜体/下划线/反显/256 色/真彩色），
 * 其余控制序列（光标移动等）直接丢弃，避免把 \x1b[33m 这类原样显示。
 */

const ANSI_RE = /\x1b\[([0-9;?]*)([a-zA-Z])/g;

const BASIC_COLORS = [
  "#000000",
  "#cc0000",
  "#00aa00",
  "#aa5500",
  "#0000cc",
  "#cc00cc",
  "#00aaaa",
  "#aaaaaa",
];
const BRIGHT_COLORS = [
  "#666666",
  "#ff5555",
  "#55ff55",
  "#ffff55",
  "#5555ff",
  "#ff55ff",
  "#55ffff",
  "#ffffff",
];

export interface AnsiStyle {
  color?: string;
  bg?: string;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  inverse?: boolean;
}

function esc(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function xterm256(n: number): string {
  if (n < 16) {
    const table = [
      "#000000",
      "#800000",
      "#008000",
      "#808000",
      "#000080",
      "#800080",
      "#008080",
      "#c0c0c0",
      "#808080",
      "#ff0000",
      "#00ff00",
      "#ffff00",
      "#0000ff",
      "#ff00ff",
      "#00ffff",
      "#ffffff",
    ];
    return table[n];
  }
  if (n < 232) {
    const v = n - 16;
    const r = Math.floor(v / 36);
    const g = Math.floor((v % 36) / 6);
    const b = v % 6;
    const conv = (x: number) => (x === 0 ? 0 : 55 + x * 40);
    return `rgb(${conv(r)},${conv(g)},${conv(b)})`;
  }
  const g = 8 + (n - 232) * 10;
  return `rgb(${g},${g},${g})`;
}

function applySgr(style: AnsiStyle, p: number[]): void {
  for (let i = 0; i < p.length; i++) {
    const c = p[i];
    if (c === 0) {
      style.color = undefined;
      style.bg = undefined;
      style.bold = false;
      style.italic = false;
      style.underline = false;
      style.inverse = false;
    } else if (c === 1) style.bold = true;
    else if (c === 22) style.bold = false;
    else if (c === 3) style.italic = true;
    else if (c === 23) style.italic = false;
    else if (c === 4) style.underline = true;
    else if (c === 24) style.underline = false;
    else if (c === 7) style.inverse = true;
    else if (c === 27) style.inverse = false;
    else if (c === 39) style.color = undefined;
    else if (c === 49) style.bg = undefined;
    else if (c >= 30 && c <= 37) style.color = BASIC_COLORS[c - 30];
    else if (c >= 90 && c <= 97) style.color = BRIGHT_COLORS[c - 90];
    else if (c >= 40 && c <= 47) style.bg = BASIC_COLORS[c - 40];
    else if (c >= 100 && c <= 107) style.bg = BRIGHT_COLORS[c - 100];
    else if (c === 38 || c === 48) {
      const next = p[i + 1];
      if (next === 5) {
        const rgb = xterm256(p[i + 2] ?? 0);
        if (c === 38) style.color = rgb;
        else style.bg = rgb;
        i += 2;
      } else if (next === 2) {
        const r = p[i + 2] ?? 0;
        const g = p[i + 3] ?? 0;
        const b = p[i + 4] ?? 0;
        const rgb = `rgb(${r},${g},${b})`;
        if (c === 38) style.color = rgb;
        else style.bg = rgb;
        i += 4;
      }
    }
  }
}

export function ansiToHtmlWithState(
  initial: AnsiStyle,
  text: string,
): { html: string; style: AnsiStyle } {
  if (!text) return { html: "", style: { ...initial } };
  const style: AnsiStyle = { ...initial };
  let out = "";
  let last = 0;
  let m: RegExpExecArray | null;
  ANSI_RE.lastIndex = 0;

  const flush = (end: number) => {
    const chunk = text.slice(last, end);
    if (!chunk) return;
    const css: string[] = [];
    if (style.inverse) {
      if (style.color) css.push(`background-color:${style.color}`);
      if (style.bg) css.push(`color:${style.bg}`);
    } else {
      if (style.color) css.push(`color:${style.color}`);
      if (style.bg) css.push(`background-color:${style.bg}`);
    }
    if (style.bold) css.push("font-weight:600");
    if (style.italic) css.push("font-style:italic");
    if (style.underline) css.push("text-decoration:underline");
    const body = esc(chunk);
    out += css.length ? `<span style="${css.join(";")}">${body}</span>` : body;
  };

  while ((m = ANSI_RE.exec(text))) {
    if (m[2] === "m") {
      flush(m.index);
      last = m.index + m[0].length;
      const params = m[1]
        ? m[1].split(";").map((x) => (x === "" ? 0 : parseInt(x, 10)))
        : [0];
      applySgr(style, params);
    } else {
      // 光标移动/清屏等非 SGR 控制序列：丢弃
      flush(m.index);
      last = m.index + m[0].length;
    }
  }
  flush(text.length);
  return { html: out, style };
}

export function ansiToHtml(text: string): string {
  return ansiToHtmlWithState({}, text).html;
}
