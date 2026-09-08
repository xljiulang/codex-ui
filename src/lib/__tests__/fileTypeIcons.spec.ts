import { describe, expect, it } from "vitest";
import {
  BUILTIN_ICON_EXTS,
  BUILTIN_ICON_NAMES,
  builtinFileIcon,
} from "../fileTypeIcons";

describe("fileTypeIcons 内置代码文件图标", () => {
  it("常见代码扩展名命中内置图标", () => {
    for (const name of [
      "a.ts",
      "b.TSX",
      "index.js",
      "App.vue",
      "main.py",
      "lib.rs",
      "App.java",
      "main.go",
      "Program.cs",
      "main.cpp",
      "util.h",
      "page.html",
      "style.scss",
      "README.md",
      "pkg.json",
      "ci.yml",
      "Cargo.toml",
      "query.sql",
      "run.sh",
      "deploy.ps1",
      "b.d.ts", // 多级扩展名按末段匹配
    ]) {
      const icon = builtinFileIcon(name);
      expect(icon, `${name} 应有内置图标`).toBeTruthy();
      expect(icon!.startsWith("data:image/svg+xml,")).toBe(true);
    }
  });

  it("特殊文件名命中内置图标（大小写不敏感）", () => {
    for (const name of ["Dockerfile", "MAKEFILE", ".gitignore", ".ENV"]) {
      expect(builtinFileIcon(name), `${name} 应有内置图标`).toBeTruthy();
    }
  });

  it("图像/视频/音频扩展名命中内置图标", () => {
    for (const name of [
      "a.png",
      "b.JPG",
      "c.jpeg",
      "d.gif",
      "e.webp",
      "f.svg",
      "g.avif",
      "v.mp4",
      "v.MKV",
      "v.mov",
      "v.webm",
      "v.3gp",
      "s.mp3",
      "s.WAV",
      "s.flac",
      "s.m4a",
    ]) {
      const icon = builtinFileIcon(name);
      expect(icon, `${name} 应有内置图标`).toBeTruthy();
      expect(icon!.startsWith("data:image/svg+xml,")).toBe(true);
    }
  });

  it("未覆盖的扩展名与无扩展名文件返回 null（走系统图标）", () => {
    expect(builtinFileIcon("config.ini")).toBeNull();
    expect(builtinFileIcon("a.txt")).toBeNull();
    expect(builtinFileIcon("data.bin")).toBeNull();
    expect(builtinFileIcon("README")).toBeNull();
    expect(builtinFileIcon(".env.local")).toBeNull(); // 仅精确匹配 .env
    expect(builtinFileIcon("Dockerfile.dev")).toBeNull();
  });

  it("覆盖清单：扩展名均带点小写，特殊文件名小写且互不重叠", () => {
    expect(BUILTIN_ICON_EXTS.every((e) => e.startsWith(".") && e === e.toLowerCase())).toBe(true);
    expect(BUILTIN_ICON_NAMES.every((n) => n === n.toLowerCase())).toBe(true);
    expect(BUILTIN_ICON_NAMES.some((n) => n.includes("."))).toBe(true);
  });

  it("data URI 可被解码且为合法 SVG（含 viewBox 与 fill）", () => {
    const uri = builtinFileIcon("main.rs")!;
    const decoded = decodeURIComponent(uri.slice("data:image/svg+xml,".length));
    expect(decoded).toContain("<svg");
    expect(decoded).toContain('viewBox="0 0 16 16"');
    expect(decoded).toContain("fill=");
  });
});
