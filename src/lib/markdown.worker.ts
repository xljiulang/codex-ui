import { marked } from "marked";

interface MarkdownRequest {
  id: number;
  text: string;
}

interface MarkdownResponse {
  id: number;
  raw: string;
}

// 离屏 Markdown 解析：主线程只负责 sanitize 与 DOM 装饰
self.onmessage = (e: MessageEvent<MarkdownRequest>) => {
  const { id, text } = e.data;
  let raw = "";
  try {
    raw = marked.parse(text, {
      async: false,
      breaks: true,
      gfm: true,
    }) as string;
  } catch {
    raw = "";
  }
  (self as unknown as Worker).postMessage({ id, raw } satisfies MarkdownResponse);
};
