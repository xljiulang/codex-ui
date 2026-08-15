import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((p: string) => `asset://${p.replace(/\\/g, "/")}`),
}));
vi.mock("../../composables/useCodex", () => ({
  setToast: vi.fn(),
  toastError: (e: unknown) => String(e),
}));

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { setToast } from "../../composables/useCodex";
import type { FileEditorTab } from "../../composables/useEditorTabs";
import {
  buildPrintHtml,
  exportMarkdownToPdf,
  pdfSuggestedName,
  resolveImageSrc,
} from "../pdfExport";

const mockedInvoke = vi.mocked(invoke);
const mockedConvertFileSrc = vi.mocked(convertFileSrc);
const mockedSetToast = vi.mocked(setToast);

function fakeTab(content: string): FileEditorTab {
  return {
    kind: "file",
    id: "t1",
    workspace: "D:\\repo",
    path: "a.md",
    title: "a.md",
    loading: false,
    error: "",
    readOnly: false,
    dirty: false,
    saving: false,
    wrap: false,
    markdownPreview: false,
    eol: "\n",
    hadBom: false,
    byteSize: content.length,
    cursor: { line: 1, col: 1 },
    status: "",
    editorState: {
      doc: { toString: () => content },
    } as unknown as FileEditorTab["editorState"],
    savedText: null,
    wrapCompartment: null,
  };
}

describe("buildPrintHtml", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedConvertFileSrc.mockClear();
    mockedSetToast.mockClear();
  });

  it("渲染标题/代码高亮/表格，并内联浅色打印样式", async () => {
    const md = [
      "# 标题",
      "",
      "正文段落",
      "",
      "```ts",
      "const x: number = 1;",
      "```",
      "",
      "| A | B |",
      "|---|---|",
      "| 1 | 2 |",
    ].join("\n");
    const html = await buildPrintHtml(md, "D:\\repo\\a.md");

    expect(html).toContain("<h1>标题</h1>");
    expect(html).toContain("<table>");
    expect(html).toContain("@page { size: A4; margin: 16mm; }");
    expect(html).toContain("Microsoft YaHei");
    expect(html).toContain("page-break-inside: avoid");
    // 代码块高亮：language-ts 保留并追加 hljs，token 有 hljs-keyword 类
    expect(html).toContain('class="language-ts hljs"');
    expect(html).toContain("hljs-keyword");
    expect(html).toContain("hljs-number");
  });

  it("本地图片重写为 asset URL，远程/data/file 按规则处理", async () => {
    const md = [
      "![相对](./img/a.png)",
      "",
      "![绝对](D:\\img\\b.png)",
      "",
      "![file](file:///D:/img/e.png)",
      "",
      "![远程](https://example.com/c.png)",
      "",
      "![data](data:image/png;base64,AAAA)",
    ].join("\n");
    const html = await buildPrintHtml(md, "D:\\repo\\docs\\a.md");

    expect(html).toContain('src="asset://D:/repo/docs/img/a.png"');
    expect(html).toContain('src="asset://D:/img/b.png"');
    expect(html).toContain('src="asset://D:/img/e.png"');
    expect(html).toContain('src="https://example.com/c.png"');
    expect(html).toContain('src="data:image/png;base64,AAAA"');
  });

  it("resolveImageSrc：锚点/未知 scheme 返回 null，路径相对 md 文件目录解析", () => {
    expect(resolveImageSrc("#anchor", "D:\\repo\\a.md")).toBeNull();
    expect(resolveImageSrc("plugin://x", "D:\\repo\\a.md")).toBeNull();
    expect(resolveImageSrc("../up.png", "D:\\repo\\docs\\a.md")).toBe(
      "asset://D:/repo/up.png",
    );
  });

  it("pdfSuggestedName 去目录与 md 扩展名", () => {
    expect(pdfSuggestedName("docs\\readme.md")).toBe("readme.pdf");
    expect(pdfSuggestedName("a.markdown")).toBe("a.pdf");
  });
});

describe("exportMarkdownToPdf", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedSetToast.mockClear();
  });

  it("成功：以当前文档内容调用导出命令并提示保存路径", async () => {
    mockedInvoke.mockResolvedValue("D:\\out\\a.pdf");
    await exportMarkdownToPdf(fakeTab("# 标题\n\n正文"));

    expect(mockedInvoke).toHaveBeenCalledWith(
      "export_markdown_pdf",
      expect.objectContaining({
        suggestedName: "a.pdf",
        initialDir: "D:\\repo",
      }),
    );
    const args = mockedInvoke.mock.calls[0][1] as { html?: string };
    expect(args.html ?? "").toContain("<h1>标题</h1>");
    expect(mockedSetToast).toHaveBeenCalledWith("已导出 PDF：D:\\out\\a.pdf");
  });

  it("取消（返回 null）：不提示", async () => {
    mockedInvoke.mockResolvedValue(null);
    await exportMarkdownToPdf(fakeTab("# hi"));
    expect(mockedSetToast).not.toHaveBeenCalled();
  });

  it("失败：toast 展示后端错误", async () => {
    mockedInvoke.mockRejectedValue("导出 PDF 失败: xxx");
    await exportMarkdownToPdf(fakeTab("# hi"));
    expect(mockedSetToast).toHaveBeenCalledWith("导出 PDF 失败: xxx");
  });

  it("空文档：不调用导出命令，直接提示", async () => {
    await exportMarkdownToPdf(fakeTab("   \n "));
    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(mockedSetToast).toHaveBeenCalledWith("文档为空，无法导出 PDF");
  });
});
