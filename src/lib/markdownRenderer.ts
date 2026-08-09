import { marked } from "marked";
import DOMPurify from "dompurify";

/** 与 MarkdownText 之前一致的 URI 白名单：http/file/mailto/盘符路径/相对路径 */
export const MARKDOWN_URI_REGEXP =
  /^(?:(?:https?|file|mailto|tel|callto|sms|cid|xmpp|matrix):|[a-z]:(?:[\\/]|%5[cC]|%2[fF])|[^a-z]|[a-z+.\-]+(?:[^a-z+.\-:]|$))/i;

/**
 * 清洗 marked 输出：
 * - 包一层 div 再 sanitize，避免 DOMPurify 剥离文档根元素 <pre>；
 *   （DOMPurify 会剥掉这个包装根 div，因此返回的 HTML 直接是顶层块元素）；
 * - 放行 file: 与盘符路径 href（本地文件链接需要）。
 */
export function sanitizeMarkdown(raw: string): string {
  return DOMPurify.sanitize(`<div>${raw}</div>`, {
    ALLOWED_URI_REGEXP: MARKDOWN_URI_REGEXP,
  });
}

function parseSync(text: string): string {
  return marked.parse(text, {
    async: false,
    breaks: true,
    gfm: true,
  }) as string;
}

interface PendingReq {
  text: string;
  resolve: (raw: string) => void;
  reject: (e: unknown) => void;
  timer: number;
}

let worker: Worker | null | undefined; // undefined=未尝试，null=不可用
let nextId = 1;
const pending = new Map<number, PendingReq>();
const WORKER_TIMEOUT = 2000;

function ensureWorker(): Worker | null {
  if (worker !== undefined) return worker;
  if (typeof Worker === "undefined") {
    worker = null;
    return null;
  }
  try {
    const w = new Worker(new URL("./markdown.worker.ts", import.meta.url), {
      type: "module",
    });
    w.onmessage = (e: MessageEvent<{ id: number; raw: string }>) => {
      const p = pending.get(e.data.id);
      if (!p) return;
      pending.delete(e.data.id);
      window.clearTimeout(p.timer);
      p.resolve(e.data.raw);
    };
    w.onerror = () => {
      // Worker 异常：标记不可用并回退同步解析
      worker = null;
      try {
        w.terminate();
      } catch {}
      const items = [...pending.values()];
      pending.clear();
      for (const p of items) {
        window.clearTimeout(p.timer);
        try {
          p.resolve(parseSync(p.text));
        } catch (e) {
          p.reject(e);
        }
      }
    };
    worker = w;
  } catch {
    worker = null;
  }
  return worker;
}

/**
 * 渲染 Markdown：优先 Web Worker 离屏解析，主线程只做 sanitize；
 * Worker 不可用/超时/出错时回退主线程同步解析。返回清洗后的 HTML。
 */
export function renderMarkdown(text: string): Promise<string> {
  return new Promise<string>((resolve, reject) => {
    const runSync = () => {
      try {
        resolve(sanitizeMarkdown(parseSync(text)));
      } catch (e) {
        reject(e);
      }
    };
    const w = ensureWorker();
    if (!w) {
      runSync();
      return;
    }
    const id = nextId++;
    const timer = window.setTimeout(() => {
      pending.delete(id);
      worker = null;
      try {
        w.terminate();
      } catch {}
      runSync();
    }, WORKER_TIMEOUT);
    pending.set(id, {
      text,
      resolve: (raw) => resolve(sanitizeMarkdown(raw)),
      reject,
      timer,
    });
    try {
      w.postMessage({ id, text });
    } catch {
      pending.delete(id);
      window.clearTimeout(timer);
      runSync();
    }
  });
}
