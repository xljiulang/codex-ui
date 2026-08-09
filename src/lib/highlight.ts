import hljs from "highlight.js/lib/core";
import type { LanguageFn } from "highlight.js";
import bash from "highlight.js/lib/languages/bash";
import c from "highlight.js/lib/languages/c";
import cpp from "highlight.js/lib/languages/cpp";
import csharp from "highlight.js/lib/languages/csharp";
import css from "highlight.js/lib/languages/css";
import diff from "highlight.js/lib/languages/diff";
import go from "highlight.js/lib/languages/go";
import html from "highlight.js/lib/languages/xml";
import ini from "highlight.js/lib/languages/ini";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import kotlin from "highlight.js/lib/languages/kotlin";
import markdown from "highlight.js/lib/languages/markdown";
import php from "highlight.js/lib/languages/php";
import powershell from "highlight.js/lib/languages/powershell";
import python from "highlight.js/lib/languages/python";
import ruby from "highlight.js/lib/languages/ruby";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import typescript from "highlight.js/lib/languages/typescript";
import yaml from "highlight.js/lib/languages/yaml";

// 只注册常用语言，控制离线包体积
const languages: [string, LanguageFn][] = [
  ["bash", bash],
  ["c", c],
  ["cpp", cpp],
  ["csharp", csharp],
  ["css", css],
  ["diff", diff],
  ["go", go],
  ["xml", html],
  ["ini", ini],
  ["java", java],
  ["javascript", javascript],
  ["json", json],
  ["kotlin", kotlin],
  ["markdown", markdown],
  ["php", php],
  ["powershell", powershell],
  ["python", python],
  ["ruby", ruby],
  ["rust", rust],
  ["sql", sql],
  ["typescript", typescript],
  ["yaml", yaml],
];
for (const [name, lang] of languages) {
  hljs.registerLanguage(name, lang);
}

export default hljs;

/** 扩展名 → highlight.js 语言（与上面注册的语言一致） */
const EXT_TO_LANG: Record<string, string> = {
  ts: "typescript",
  mts: "typescript",
  cts: "typescript",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  py: "python",
  rs: "rust",
  cs: "csharp",
  c: "c",
  h: "c",
  cpp: "cpp",
  cc: "cpp",
  cxx: "cpp",
  hpp: "cpp",
  go: "go",
  java: "java",
  kt: "kotlin",
  kts: "kotlin",
  json: "json",
  md: "markdown",
  markdown: "markdown",
  yml: "yaml",
  yaml: "yaml",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  ps1: "powershell",
  psm1: "powershell",
  sql: "sql",
  css: "css",
  html: "xml",
  htm: "xml",
  xml: "xml",
  svg: "xml",
  ini: "ini",
  cfg: "ini",
  php: "php",
  rb: "ruby",
  diff: "diff",
  patch: "diff",
};

/** 根据文件路径推断 highlight.js 语言；无法识别返回 null */
export function languageFromPath(path: string): string | null {
  const name = path.split(/[\\/]/).pop() ?? "";
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return null;
  return EXT_TO_LANG[name.slice(dot + 1).toLowerCase()] ?? null;
}
