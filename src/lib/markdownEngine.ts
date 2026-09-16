import { Marked } from "marked";

/**
 * 把尖括号内容转义为字面文本。`&` 必须最先替换，
 * 否则后续插入的实体会被二次转义。
 */
export function escapeMarkdownHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/**
 * 全局唯一的 Markdown 引擎：气泡、计划卡片、文件预览、PDF 导出与纯文本提取
 * 共用同一份配置（gfm + breaks），保证各处语义一致。
 *
 * 关键约定：源码里的原始 HTML（`<dd>`、`<ABCD>`、`<img src=…>`、注释、CDATA、
 * 处理指令等）一律转义为字面文本——这些内容被 marked 当作 HTML 透传后，
 * 经 DOMPurify 清洗会变成没有可见文本的元素（整个气泡空白）或只剩正文
 * （`a<b>c` → `ac`），用户看不到自己写的内容。行内/围栏代码块内的标签本来就是
 * 代码文本，不受影响；Markdown 语法的链接、图片、表格等仍照常渲染。
 */
const engine = new Marked({ gfm: true, breaks: true });
engine.use({
  renderer: {
    html({ text }) {
      return escapeMarkdownHtml(text);
    },
  },
});

/** 解析 Markdown 为 HTML（原始 HTML 已转义，调用方仍须清洗） */
export function parseMarkdown(text: string): string {
  return engine.parse(text, { async: false }) as string;
}

/** 词法分析：与渲染同一实例，供纯文本提取使用（不渲染 HTML） */
export function lexMarkdown(text: string) {
  return engine.lexer(text);
}
