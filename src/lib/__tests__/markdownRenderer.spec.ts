import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { renderMarkdown } from "../markdownRenderer";

// happy-dom 的 Worker 不会真正解析消息；测试固定走同步回退路径
const origWorker = globalThis.Worker;

/** 渲染结果的可见文本（折叠空白），用于断言「用户写的内容没有消失」 */
function visibleText(html: string): string {
  const el = document.createElement("div");
  el.innerHTML = html;
  return (el.textContent ?? "").replace(/\s+/g, " ").trim();
}

describe("renderMarkdown（同步回退路径）", () => {
  beforeEach(() => {
    (globalThis as Record<string, unknown>).Worker = undefined;
  });

  afterEach(() => {
    (globalThis as Record<string, unknown>).Worker = origWorker;
  });

  it("普通文本渲染为清洗后的 HTML", async () => {
    const html = await renderMarkdown("hello **world**");
    expect(html).toContain("<strong>world</strong>");
  });

  it("消息以代码块开头时保留根 <pre>（回归：DOMPurify 根元素修复）", async () => {
    const html = await renderMarkdown("```js\nconst x = 1;\n```");
    expect(html).toContain("<pre>");
    expect(html).toContain('class="language-js"');
  });

  it("file: 链接 href 不被剥离", async () => {
    const html = await renderMarkdown("[本地](file:///D:/a.txt)");
    expect(html).toContain('href="file:///D:/a.txt"');
  });

  it("盘符路径链接 href 不被剥离（marked 编码为 %5C）", async () => {
    const html = await renderMarkdown("[计划.md](D:\\codex\\a.md)");
    expect(html).toContain('href="D:%5Ccodex%5Ca.md"');
  });

  it("尖括号内容转义为字面文本且不生成元素（回归：<dd>/<ABCD> 渲染成空白）", async () => {
    const literal = [
      "<dd>",
      "<foo>",
      "<ABCD>",
      "<T>",
      "<user_instructions>",
      "</dd>",
      "<hr>",
      "<br>",
      '<img src="https://e.com/a.png">',
      "<!-- 注释 -->",
      "<![CDATA[x]]>",
      "<?php echo 1;?>",
    ];
    for (const src of literal) {
      const html = await renderMarkdown(src);
      expect(visibleText(html)).toBe(src);
      const el = document.createElement("div");
      el.innerHTML = html;
      expect(el.querySelector("*")).toBeNull();
    }
  });

  it("嵌在句子里的尖括号内容不丢标签、不整段消失", async () => {
    expect(visibleText(await renderMarkdown("a<b>c"))).toBe("a<b>c");
    expect(
      visibleText(await renderMarkdown("把 <user_message> 里的东西改掉")),
    ).toBe("把 <user_message> 里的东西改掉");
    const html = await renderMarkdown('<a href="D:/a.txt">x</a>');
    expect(visibleText(html)).toBe('<a href="D:/a.txt">x</a>');
    const el = document.createElement("div");
    el.innerHTML = html;
    expect(el.querySelector("a")).toBeNull();
  });

  it("marked 本就不当 HTML 的尖括号文本保持原样", async () => {
    for (const src of ["<中文标签>", "<1+2>", "<=>", "x < y", "<>"]) {
      expect(visibleText(await renderMarkdown(src))).toBe(src);
    }
  });

  it("代码块与行内代码里的标签仍是代码文本（不双转义）", async () => {
    const fenced = await renderMarkdown("```\n<dd>\n```");
    expect(fenced).toContain("<pre>");
    expect(visibleText(fenced)).toBe("<dd>");
    const inline = await renderMarkdown("`<dd>`");
    expect(inline).toContain("<code>");
    expect(visibleText(inline)).toBe("<dd>");
  });
});
