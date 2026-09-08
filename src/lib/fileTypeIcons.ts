/** 内置代码文件图标：常见代码/配置类型优先于系统图标，VS 风格彩色小图形 */
import { extOf } from "./preview";

/** 包装为 SVG data URI（现有渲染方均以 <img :src> 显示，矢量在 14/16px 下更清晰） */
function svg(inner: string): string {
  return `data:image/svg+xml,${encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">${inner}</svg>`,
  )}`;
}

/** 描边路径（圆角端点，统一笔触风格） */
function stroke(color: string, d: string, width = 1.4): string {
  return `<path d="${d}" fill="none" stroke="${color}" stroke-width="${width}" stroke-linecap="round" stroke-linejoin="round"/>`;
}

/** 圆角方形底徽 + 白色内容 */
function badge(color: string, inner: string): string {
  return `<rect x="1.5" y="1.5" width="13" height="13" rx="2.6" fill="${color}"/>${inner}`;
}

/** 齿轮（配置类：Rust / .env） */
function gearGlyph(color: string): string {
  const tooth = `<rect x="7.25" y=".9" width="1.5" height="3.2" rx=".6"/>`;
  const teeth = [0, 45, 90, 135, 180, 225, 270, 315]
    .map((a) => (a === 0 ? tooth : `<g transform="rotate(${a} 8 8)">${tooth}</g>`))
    .join("");
  return `<g fill="${color}"><path fill-rule="evenodd" d="M8 3.7A4.3 4.3 0 1 0 8 12.3 4.3 4.3 0 0 0 8 3.7zm0 2.4a1.9 1.9 0 1 1 0 3.8 1.9 1.9 0 0 1 0-3.8z"/>${teeth}</g>`;
}

/** 花括号对（JSON / CSS 系） */
function bracesGlyph(color: string): string {
  return svg(
    stroke(
      color,
      "M6.5 3.1c-1.4 0-2.1.8-2.1 2.2v1c0 .6-.4 1.1-1 1.2v.9c.6.1 1 .6 1 1.2v1c0 1.4.7 2.2 2.1 2.2",
    ) +
      stroke(
        color,
        "M9.5 3.1c1.4 0 2.1.8 2.1 2.2v1c0 .6.4 1.1 1 1.2v.9c-.6.1-1 .6-1 1.2v1c0 1.4-.7 2.2-2.1 2.2",
      ),
  );
}

/* TypeScript：蓝底白 TS */
const TS = svg(
  badge(
    "#3178C6",
    stroke("#fff", "M3.7 4.7h5.4M6.4 4.7v7", 1.3) +
      stroke("#fff", "M12.5 4.7H10v2.9h2.2v2.9H10", 1.3),
  ),
);

/* JavaScript：黄底黑 JS */
const JS = svg(
  badge(
    "#F7DF1E",
    stroke("#1E1E1E", "M4.9 4.7v5.2a1.5 1.5 0 0 1-1.5 1.5H2.9", 1.3) +
      stroke("#1E1E1E", "M12.6 4.7H10v2.9h2.2v2.9H10", 1.3),
  ),
);

/* Vue：双层 V */
const VUE = svg(
  `<path d="M1.4 2.2h3.3L8 8.4l3.3-6.2h3.3L8 14.2z" fill="#41B883"/>` +
    `<path d="M5.1 2.2h2L8 4.6l1-2.4h1.9L8 8.4z" fill="#35495E"/>`,
);

/* Python：蓝黄双蛇（下半为上半旋转 180°） */
const PY_SNAKE =
  "M8.2 1.5c-2.4 0-3.6 1-3.6 2.6v2.4h3.5v1H3.4c-1.6 0-2.5 1.1-2.5 2.8 0 1.6.8 2.7 2.3 2.7h1v-2.1c0-1.5 1.2-2.7 2.7-2.7h3.4c1.2 0 2.1-.9 2.1-2.1V4.1c0-1.7-1.3-2.6-3.2-2.6z";
const PY = svg(
  `<path d="${PY_SNAKE}" fill="#3776AB"/><circle cx="6.3" cy="3.6" r=".8" fill="#fff"/>` +
    `<g transform="rotate(180 8 8)"><path d="${PY_SNAKE}" fill="#FFD43B"/><circle cx="6.3" cy="3.6" r=".8" fill="#fff"/></g>`,
);

/* Ruby：红宝石切面 */
const RB = svg(
  `<path d="M4.6 2.8h6.8L14 6.4 8 13.4 2 6.4z" fill="#CC342D"/>` +
    `<path d="M2 6.4h12M8 13.4 4.6 2.8M8 13.4 11.4 2.8" stroke="#fff" stroke-width=".7" opacity=".55" fill="none"/>`,
);

/* PHP：紫椭圆 + php 字样 */
const PHP = svg(
  `<ellipse cx="8" cy="8" rx="7.2" ry="4.4" fill="#777BB4"/>` +
    stroke("#fff", "M4.1 5.9v4.4", 1) +
    stroke("#fff", "M4.1 6.6h.9a1.2 1.2 0 1 1 0 2.4h-.9", 1) +
    stroke("#fff", "M7.5 5.9v4.4", 1) +
    stroke("#fff", "M7.5 8.1c0-.9.6-1.6 1.4-1.6s1.3.6 1.3 1.4v2.4", 1) +
    stroke("#fff", "M11.6 5.9v4.4", 1) +
    stroke("#fff", "M11.6 6.6h.9a1.2 1.2 0 1 1 0 2.4h-.9", 1),
);

/* Java：咖啡杯 + 热气 */
const JAVA = svg(
  `<path d="M3.4 8.7h7.3l-.9 4a1.9 1.9 0 0 1-1.8 1.5H6.1a1.9 1.9 0 0 1-1.8-1.5z" fill="#E76F00"/>` +
    `<path d="M10.8 9.5h.9a1.7 1.7 0 0 1 0 3.4h-1.4" fill="none" stroke="#E76F00" stroke-width="1.1" stroke-linecap="round"/>` +
    `<path d="M6.3 6.4c.9-.9.9-1.9.2-3M9.1 6.4c.9-.9.9-1.9.2-3" fill="none" stroke="#E76F00" stroke-width="1" stroke-linecap="round"/>`,
);

/* Go：GO 字标 */
const GO = svg(
  stroke("#00ADD8", "M6.9 5.4A2.8 2.8 0 1 0 8.4 8H5.8") +
    `<circle cx="12" cy="8" r="2.5" fill="none" stroke="#00ADD8" stroke-width="1.4"/>`,
);

/* Rust：橙色齿轮 */
const RS = svg(gearGlyph("#CE422B"));

/* Dart：蓝青双折纸 */
const DART = svg(
  `<path d="M6.6 1.9h3.2l4.3 4.3-4.3 4.3-3.2-3.2z" fill="#00A8E1"/>` +
    `<path d="M6.6 1.9 2.3 6.2l1.9 1.9 4.3-4.3z" fill="#00569E"/>` +
    `<path d="M9.8 10.2 8.3 11.7 4.3 7.7 2.5 9.5l5.8 5.8 4.3-4.3z" fill="#015B95"/>`,
);

/* Lua：蓝色月牙 + 星点 */
const LUA = svg(
  `<path d="M9.3 2.3a5.7 5.7 0 1 0 4.4 4.4 4.5 4.5 0 0 1-4.4-4.4z" fill="#2C2D72"/>` +
    `<circle cx="12.6" cy="3.4" r="1" fill="#2C2D72"/>`,
);

/* Swift：橙色飞燕 */
const SWIFT = svg(
  `<path d="M11.9 2.3c.7 1.1 1 2.3.7 3.5 1.1 1.6 1.4 3.2.8 4.8-1.5 2.5-4.4 3.8-7.4 3.4-1.7-.2-3.2-1-4.3-2.1 1.9.3 3.7-.2 5-1.4-1.3-.7-2.4-1.8-3.1-3.4.9.6 2 1 3 1.1C5.3 7.3 4.3 5.7 4.1 3.8c1.2.8 2.5 1.4 3.9 1.6.3-1.6 1.9-3.1 3.9-3.1z" fill="#F05138"/>`,
);

/* C / C++ / C#：蓝系底徽 + 字符 */
const C_ICON = svg(badge("#03599C", stroke("#fff", "M10.9 5.9a3.1 3.1 0 1 0 0 4.2", 1.5)));
const CPP = svg(
  badge(
    "#00599C",
    stroke("#fff", "M9.4 6.2a2.5 2.5 0 1 0 0 3.6", 1.3) +
      stroke("#fff", "M10.9 6.5v2.6M9.6 7.8h2.6", 1.1) +
      stroke("#fff", "M12.7 6.5v2.6M11.4 7.8h2.6", 1.1),
  ),
);
const CS = svg(
  badge(
    "#5C2D91",
    stroke("#fff", "M9.3 6.2a2.5 2.5 0 1 0 0 3.6", 1.3) +
      stroke("#fff", "M11.3 5.9l-.9 4M13 5.9l-.9 4M10.6 7.2h3M10.4 8.6h3", 1.1),
  ),
);

/* Kotlin：渐变菱形 K */
const KT = svg(
  `<defs><linearGradient id="ktg" x1="1" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#E44857"/><stop offset=".47" stop-color="#C711E1"/><stop offset="1" stop-color="#7F52FF"/></linearGradient></defs>` +
    `<path d="M2 2h12L8 8l6 6H2z" fill="url(#ktg)"/>`,
);

/* HTML：橙色盾牌 + 尖括号 */
const HTML = svg(
  `<path d="M2.6 1.8h10.8l-1 11L8 14.6l-4.4-1.8z" fill="#E44D26"/>` +
    `<path d="M5.7 5.6 4 7.8l1.7 2.2M10.3 5.6l1.7 2.2-1.7 2.2" stroke="#fff" stroke-width="1.1" fill="none" stroke-linecap="round" stroke-linejoin="round"/>`,
);

/* XML：橙色尖括号 + 斜杠 */
const XML = svg(
  stroke("#E44D26", "M5.6 4.4 2.9 8l2.7 3.6") +
    stroke("#E44D26", "M10.4 4.4 13.1 8l-2.7 3.6") +
    stroke("#E44D26", "M8.9 3.4 7.1 12.6"),
);

/* Markdown：蓝底 M↓ */
const MD = svg(
  badge(
    "#519ABA",
    stroke("#fff", "M3.4 10.8V5.6l2.2 2.5 2.2-2.5v5.2", 1.2) +
      stroke("#fff", "M10.3 5.6v4.1M8.8 8.2l1.5 1.6 1.5-1.6", 1.2),
  ),
);

/* YAML / TOML：底徽字标 */
const YML = svg(badge("#6B4E9E", stroke("#fff", "M5 4.6 8 8l3-3.4M8 8v3.8", 1.3)));
const TOML = svg(badge("#9C4121", stroke("#fff", "M4.7 4.9h6.6M8 4.9v6.6", 1.3)));

/* SQL：数据库圆柱 */
const SQL = svg(
  `<path d="M2.8 4.7v6.6c0 1.1 2.3 2 5.2 2s5.2-.9 5.2-2V4.7" fill="#336791"/>` +
    `<ellipse cx="8" cy="4.7" rx="5.2" ry="2" fill="#5B8CB8"/>`,
);

/* 脚本：终端提示符底徽（Shell 灰 / PowerShell 深蓝） */
const SH_INNER = stroke("#fff", "M4.2 5.2 7 8l-2.8 2.8M8.6 10.8h3.2", 1.3);
const SH = svg(badge("#4A4A4A", SH_INNER));
const PS1 = svg(badge("#012456", SH_INNER));

/* Docker：集装箱船 */
const DOCKER = svg(
  `<g fill="#2496ED"><rect x="5.6" y="5.4" width="1.9" height="1.9" rx=".3"/><rect x="7.7" y="5.4" width="1.9" height="1.9" rx=".3"/><rect x="9.8" y="5.4" width="1.9" height="1.9" rx=".3"/><rect x="7.7" y="3.3" width="1.9" height="1.9" rx=".3"/></g>` +
    `<path d="M1.6 8.4h12c-.2 2.6-2.3 4.6-5.6 4.6-3.1 0-5.4-1.8-6.4-4.6z" fill="#2496ED"/>`,
);

/* Makefile：灰底 M */
const MAKE = svg(badge("#6D8086", stroke("#fff", "M5.6 11.2V5L8 8.2 10.4 5v6.2", 1.3)));

/* Git 点文件：橙色分支 */
const GIT = svg(
  `<g fill="none" stroke="#F14E32" stroke-width="1.3"><path d="M4.5 5.3v5.4M11.6 7.2c0 2.3-2.4 3.1-5 3.4"/><circle cx="4.5" cy="3.6" r="1.7"/><circle cx="4.5" cy="12.4" r="1.7"/><circle cx="11.6" cy="5.5" r="1.7"/></g>`,
);

/* .env：绿色齿轮 */
const ENV = svg(gearGlyph("#8BC34A"));

const JSON_ICON = bracesGlyph("#CBCB41");
const CSS = bracesGlyph("#1572B6");
const SCSS = bracesGlyph("#CF649A");
const LESS = bracesGlyph("#2B5E84");

/* 图像：黛青底徽 + 双层远山 + 太阳（山水画风格） */
const IMAGE = svg(
  badge(
    "#2E7D8C",
    `<circle cx="10.7" cy="4.8" r="1.3" fill="#fff" opacity=".92"/>` +
      `<path d="M2.3 12.3 5.5 7 7.2 9.6 8.7 7.7 13 12.3z" fill="#fff" opacity=".5"/>` +
      `<path d="M4.4 12.3 7.4 8.2 9 10.3 10.2 9 13.7 12.3z" fill="#fff"/>`,
  ),
);

/* 视频：红色底徽 + 白色播放三角 */
const VIDEO = svg(
  badge("#E53935", `<path d="M6.4 4.9 11 8l-4.6 3.1z" fill="#fff"/>`),
);

/* 音频：紫色底徽 + 白色音符 */
const AUDIO = svg(
  badge(
    "#7E57C2",
    `<circle cx="6.2" cy="11.1" r="1.7" fill="#fff"/>` +
      `<rect x="7.4" y="4.1" width="1.3" height="7.1" rx=".65" fill="#fff"/>` +
      `<path d="M8.7 4.1 11.8 3v2.5L8.7 6.6z" fill="#fff"/>`,
  ),
);

/** 扩展名（小写，不含点）→ 图标；未列出的走系统图标 */
const EXT_ICONS: Record<string, string> = {
  ts: TS,
  tsx: TS,
  mts: TS,
  cts: TS,
  js: JS,
  jsx: JS,
  mjs: JS,
  cjs: JS,
  vue: VUE,
  py: PY,
  pyi: PY,
  pyw: PY,
  rb: RB,
  erb: RB,
  php: PHP,
  java: JAVA,
  go: GO,
  rs: RS,
  dart: DART,
  lua: LUA,
  swift: SWIFT,
  c: C_ICON,
  h: C_ICON,
  cpp: CPP,
  cc: CPP,
  cxx: CPP,
  hpp: CPP,
  hh: CPP,
  hxx: CPP,
  cs: CS,
  kt: KT,
  kts: KT,
  html: HTML,
  htm: HTML,
  xml: XML,
  css: CSS,
  scss: SCSS,
  sass: SCSS,
  less: LESS,
  md: MD,
  markdown: MD,
  json: JSON_ICON,
  jsonc: JSON_ICON,
  json5: JSON_ICON,
  yml: YML,
  yaml: YML,
  toml: TOML,
  sql: SQL,
  sh: SH,
  bash: SH,
  zsh: SH,
  bat: SH,
  cmd: SH,
  ps1: PS1,
  psm1: PS1,
  psd1: PS1,
  // 图像
  png: IMAGE,
  jpg: IMAGE,
  jpeg: IMAGE,
  gif: IMAGE,
  webp: IMAGE,
  bmp: IMAGE,
  svg: IMAGE,
  ico: IMAGE,
  avif: IMAGE,
  // 视频
  mp4: VIDEO,
  webm: VIDEO,
  mkv: VIDEO,
  mov: VIDEO,
  avi: VIDEO,
  m4v: VIDEO,
  ogv: VIDEO,
  mpg: VIDEO,
  mpeg: VIDEO,
  wmv: VIDEO,
  flv: VIDEO,
  "3gp": VIDEO,
  // 音频
  mp3: AUDIO,
  wav: AUDIO,
  ogg: AUDIO,
  oga: AUDIO,
  flac: AUDIO,
  m4a: AUDIO,
  aac: AUDIO,
  opus: AUDIO,
  wma: AUDIO,
  mid: AUDIO,
  midi: AUDIO,
  aiff: AUDIO,
  ape: AUDIO,
};

/** 特殊文件名（小写）→ 图标：无扩展名/点文件的常见配置 */
const NAME_ICONS: Record<string, string> = {
  dockerfile: DOCKER,
  makefile: MAKE,
  ".gitignore": GIT,
  ".gitattributes": GIT,
  ".gitmodules": GIT,
  ".env": ENV,
};

/**
 * 按文件名查内置图标：命中返回 SVG data URI，未命中返回 null
 * （调用方回退到系统图标链路）。大小写不敏感。
 */
export function builtinFileIcon(name: string): string | null {
  const lower = name.toLowerCase();
  const byName = NAME_ICONS[lower];
  if (byName) return byName;
  const ext = extOf(lower);
  return ext ? (EXT_ICONS[ext] ?? null) : null;
}

/** 内置图标覆盖的扩展名（含点，小写），供缓存键归一使用 */
export const BUILTIN_ICON_EXTS = Object.keys(EXT_ICONS).map((e) => `.${e}`);

/** 内置图标覆盖的特殊文件名（小写） */
export const BUILTIN_ICON_NAMES = Object.keys(NAME_ICONS);
