import { describe, expect, it } from "vitest";
import { ansiToHtml } from "../ansi";

describe("ANSI 终端输出渲染", () => {
  it("纯文本原样返回（HTML 转义）", () => {
    expect(ansiToHtml("hello <script>alert(1)</script>")).toBe(
      "hello &lt;script&gt;alert(1)&lt;/script&gt;",
    );
  });

  it("颜色转义渲染为 span，reset 后恢复", () => {
    expect(ansiToHtml("\x1b[33mwarning\x1b[0m ok")).toBe(
      '<span style="color:#aa5500">warning</span> ok',
    );
  });

  it("组合样式（加粗+绿色）", () => {
    expect(ansiToHtml("\x1b[1;32mPath\x1b[0m")).toBe(
      '<span style="color:#00aa00;font-weight:600">Path</span>',
    );
  });

  it("真彩色 38;2;r;g;b", () => {
    expect(ansiToHtml("\x1b[38;2;255;0;0mred\x1b[0m")).toBe(
      '<span style="color:rgb(255,0,0)">red</span>',
    );
  });

  it("非 SGR 控制序列被丢弃", () => {
    expect(ansiToHtml("a\x1b[2Jb")).toBe("ab");
  });
});
