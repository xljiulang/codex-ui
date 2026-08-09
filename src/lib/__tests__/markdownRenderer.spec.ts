import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { renderMarkdown } from "../markdownRenderer";

// happy-dom 的 Worker 不会真正解析消息；测试固定走同步回退路径
const origWorker = globalThis.Worker;

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
});
