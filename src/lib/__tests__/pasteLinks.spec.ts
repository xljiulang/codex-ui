import { describe, expect, it } from "vitest";
import { linksToPlainText } from "../pasteLinks";

describe("linksToPlainText", () => {
  it("普通链接 → 标题 + 空格 + URL", () => {
    expect(
      linksToPlainText('<a href="https://a.b">标题</a>'),
    ).toBe("标题 https://a.b");
  });

  it("带路径/query/hash 的 URL 原样保留", () => {
    expect(
      linksToPlainText('<a href="https://a.b/x?q=1#sec">文档</a>'),
    ).toBe("文档 https://a.b/x?q=1#sec");
  });

  it("空标题/纯空白标题 → 只保留 URL", () => {
    expect(linksToPlainText('<a href="https://a.b"></a>')).toBe(
      "https://a.b",
    );
    expect(linksToPlainText('<a href="https://a.b">  </a>')).toBe(
      "https://a.b",
    );
  });

  it("href 首尾空白被剔除", () => {
    expect(
      linksToPlainText('<a href="  https://a.b  ">标题</a>'),
    ).toBe("标题 https://a.b");
  });

  it("多链接连续各自转换", () => {
    expect(
      linksToPlainText(
        '<a href="https://a.b">A</a> 和 <a href="https://c.d">C</a>',
      ),
    ).toBe("A https://a.b 和 C https://c.d");
  });

  it("包裹在段落/列表里的链接：其余 HTML 标签保留", () => {
    expect(
      linksToPlainText(
        '<p>看 <a href="https://a.b">文档</a> 吧</p><ul><li><a href="https://c.d">项</a></li></ul>',
      ),
    ).toBe(
      '<p>看 文档 https://a.b 吧</p><ul><li>项 https://c.d</li></ul>',
    );
  });

  it("嵌套链接：内层与外层都非 a 的文本保留", () => {
    expect(
      linksToPlainText(
        '<a href="https://a.b"><strong>粗</strong>标题</a>',
      ),
    ).toBe("粗标题 https://a.b");
  });

  it("mailto: 等 URL 原样输出", () => {
    expect(linksToPlainText('<a href="mailto:a@b.c">联系</a>')).toBe(
      "联系 mailto:a@b.c",
    );
  });

  it("无 <a> 的 HTML → 原样返回", () => {
    const html = "<p>纯文本<b>粗体</b></p>";
    expect(linksToPlainText(html)).toBe(html);
  });

  it("内含文本子节点与空格的标题压平为单空格", () => {
    expect(
      linksToPlainText('<a href="https://a.b">  多   空格 \n 标题 </a>'),
    ).toBe("多 空格 标题 https://a.b");
  });
});
