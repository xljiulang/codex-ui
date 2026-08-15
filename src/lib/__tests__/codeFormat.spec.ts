import { afterEach, describe, expect, it, vi } from "vitest";

import {
  FORMATTABLE_EXTS,
  formatDoc,
  isFormattablePath,
} from "../codeFormat";

describe("codeFormat 扩展名识别", () => {
  it("覆盖 Prettier / XML / 缩进重排三组扩展名", () => {
    for (const ext of [
      "js", "jsx", "ts", "tsx", "mjs", "cjs", "mts", "cts",
      "json", "jsonc", "css", "html", "htm", "yml", "yaml", "md", "markdown",
      "xml", "svg",
      "py", "rs", "c", "h", "cpp", "go", "java", "kt", "cs", "php", "sql",
      "sh", "ps1", "rb", "ini", "cfg", "diff",
    ]) {
      expect(FORMATTABLE_EXTS.has(ext), ext).toBe(true);
      expect(isFormattablePath(`D:\\repo\\a.${ext}`), ext).toBe(true);
    }
  });

  it("不支持的扩展名不显示格式化", () => {
    expect(isFormattablePath("D:\\repo\\a.txt")).toBe(false);
    expect(isFormattablePath("D:\\repo\\noext")).toBe(false);
    expect(isFormattablePath("D:\\repo\\a.JSON")).toBe(true); // 大小写不敏感
  });
});

describe("codeFormat XML 美化", () => {
  it("单行紧凑 XML 展开为多行并缩进", async () => {
    const res = await formatDoc(
      "a.xml",
      '<root><a>1</a><b><c>x</c></b></root>',
    );
    expect(res).toEqual({
      ok: true,
      text: "<root>\n    <a>1</a>\n    <b>\n        <c>x</c>\n    </b>\n</root>",
    });
  });

  it("保留属性、注释、DOCTYPE、CDATA 与自闭合形式", async () => {
    const doc = [
      '<?xml version="1.0"?>',
      '<!DOCTYPE root SYSTEM "root.dtd">',
      "<!-- top -->",
      '<root a="1"><child/><![CDATA[ raw <x> ]]><empty></empty></root>',
    ].join("\n");
    const res = await formatDoc("a.xml", doc);
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect("text" in res).toBe(true);
    if (!("text" in res)) return;
    expect(res.text).toContain('<?xml version="1.0"?>');
    expect(res.text).toContain('<!DOCTYPE root SYSTEM "root.dtd">');
    expect(res.text).toContain("<!-- top -->");
    expect(res.text).toContain('a="1"');
    expect(res.text).toContain("<![CDATA[ raw <x> ]]>");
    expect(res.text).toContain("    <child/>");
    expect(res.text).toContain("<empty></empty>");
  });

  it("混合文本内容保持单行原文", async () => {
    const res = await formatDoc(
      "a.xml",
      "<p>Hello <b>world</b>!</p>",
    );
    // 已为单行、无需重排：返回 unchanged（不产生编辑）
    expect(res).toEqual({ ok: true, unchanged: true });
  });

  it("错误 XML 返回错误消息", async () => {
    const res = await formatDoc("a.xml", "<root><a></root>");
    expect(res).toEqual({ ok: false, message: "XML 语法错误，无法格式化" });
  });

  it("CRLF 与结尾换行保持", async () => {
    const res = await formatDoc(
      "a.xml",
      "<root>\r\n  <a>1</a>\r\n</root>\r\n",
    );
    expect(res).toEqual({
      ok: true,
      text: "<root>\r\n    <a>1</a>\r\n</root>\r\n",
    });
  });

  it("已格式化文档返回 unchanged", async () => {
    const res = await formatDoc("a.xml", "<root>\n    <a>1</a>\n</root>");
    expect(res).toEqual({ ok: true, unchanged: true });
  });
});

describe("codeFormat 缩进重排（非 Prettier 语言）", () => {
  it("单行内容保持原样（不展开）", async () => {
    const res = await formatDoc("a.go", "func f() { if x { return 1 } }");
    expect(res).toEqual({ ok: true, unchanged: true });
  });

  it("Python 续行缩进对齐", async () => {
    const res = await formatDoc(
      "a.py",
      "def f(a):\n  if a:\n      return [1,\n2,\n3]",
    );
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect("text" in res).toBe(true);
    if (!("text" in res)) return;
    expect(res.text).toContain("        return [1,");
  });
});

describe("codeFormat Prettier 路径", () => {
  afterEach(() => {
    vi.resetModules();
  });

  it("非法 JSON 返回错误消息", async () => {
    const res = await formatDoc("a.json", '{"a": }');
    expect(res).toEqual({ ok: false, message: "JSON 语法错误，无法格式化" });
  });

  it("JSONC 注释被保留（Prettier json 解析器）", async () => {
    const res = await formatDoc(
      "a.jsonc",
      '{\n// comment\n"a":1,\n}',
    );
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect("text" in res).toBe(true);
    if (!("text" in res)) return;
    expect(res.text).toContain("// comment");
    expect(res.text).toContain('"a": 1');
  });

  it("JSON 规范化：短对象保持单行并加空格（Prettier 打印宽度内）", async () => {
    const res = await formatDoc("a.json", '{"a":1,"b":[1,2]}');
    expect(res).toEqual({ ok: true, text: '{ "a": 1, "b": [1, 2] }' });
  });
});
