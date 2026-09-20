import DOMPurify from "dompurify";
import { parseMarkdown } from "./markdownEngine";

/** 与 MarkdownText 之前一致的 URI 白名单：http/file/mailto/盘符路径/相对路径 */
export const MARKDOWN_URI_REGEXP =
  /^(?:(?:https?|file|mailto|tel|callto|sms|cid|xmpp|matrix):|[a-z]:(?:[\\/]|%5[cC]|%2[fF])|[^a-z]|[a-z+.\-]+(?:[^a-z+.\-:]|$))/i;

/**
 * 清洗 marked 输出：
 * - 包一层 div 再 sanitize，避免 DOMPurify 剥离文档根元素 <pre>；
 *   （DOMPurify 会剥掉这个包装根 div，因此返回的 HTML 直接是顶层块元素）；
 * - 放行 file: 与盘符路径 href（本地文件链接需要）。
 * 原始 HTML 已在 markdownEngine 里转义为字面文本，此处是纵深防御。
 */
export function sanitizeMarkdown(raw: string): string {
  return DOMPurify.sanitize(`<div>${raw}</div>`, {
    ALLOWED_URI_REGEXP: MARKDOWN_URI_REGEXP,
  });
}

function parseSync(text: string): string {
  return parseMarkdown(text);
}

interface PendingReq {
  text: string;
  resolve: (raw: string) => void;
  reject: (e: unknown) => void;
}

let worker: Worker | null | undefined; // undefined=未尝试，null=不可用
let nextId = 1;
const pending = new Map<number, PendingReq>();

/**
 * Worker 停滞判定（毫秒）：只在「队列里仍有待处理请求」且「这段时间内没有任何响应」时才算卡死。
 * 不能用「单请求超时」：大历史会话一次挂载上千个 MarkdownText，请求在 Worker 里排队，
 * 单个请求等上几秒完全正常；按请求超时会把仍在正常排队的 Worker 误判为不可用并整体停用，
 * 之后每个待处理请求各自回退主线程同步解析，造成长时间卡顿、正文持续长高。
 */
const WORKER_STALL_MS = 5000;
/** 停滞回退时每批最多消耗的毫秒数（批间让出主线程，避免一次性冻住 UI） */
const FALLBACK_BATCH_MS = 16;
/** 停滞回退时每批最多结算的请求条数 */
const FALLBACK_BATCH_MAX = 8;
let stallTimer: number | undefined;

/** 单调时钟（performance 不可用时回落到 Date） */
function nowMs(): number {
  return typeof performance !== "undefined" &&
    typeof performance.now === "function"
    ? performance.now()
    : Date.now();
}

/** 待渲染 Markdown 条数（会话历史预热据此判断正文是否已全部落地） */
export function pendingMarkdownCount(): number {
  return pending.size;
}

function clearStallTimer() {
  if (stallTimer !== undefined) {
    window.clearTimeout(stallTimer);
    stallTimer = undefined;
  }
}

/** 重置停滞看门狗：postMessage 与 Worker 每次响应都算「还活着」 */
function armStallTimer() {
  clearStallTimer();
  stallTimer = window.setTimeout(() => {
    stallTimer = undefined;
    stallAndFallback();
  }, WORKER_STALL_MS);
}

/** Worker 判定卡死：停用 Worker，并把全部待处理请求按 FIFO 分帧回退主线程解析 */
function stallAndFallback() {
  const dead = worker ?? null;
  worker = null;
  if (dead) {
    try {
      dead.terminate();
    } catch {}
  }
  drainPendingSync();
}

/**
 * 按 FIFO 分帧结算全部待处理请求：每批受 FALLBACK_BATCH_MS / FALLBACK_BATCH_MAX 约束，
 * 批间用 setTimeout(0) 让出主线程；批内仍按入队顺序 resolve，promise 语义不变
 * （map 内登记的是「接原始 HTML 再 sanitize」的包装，故此处只传原始解析结果）。
 */
function drainPendingSync() {
  const items = [...pending.entries()];
  pending.clear();
  if (items.length === 0) return;
  let i = 0;
  const runBatch = () => {
    const started = nowMs();
    let count = 0;
    while (i < items.length && count < FALLBACK_BATCH_MAX) {
      const [, p] = items[i++];
      count++;
      try {
        p.resolve(parseSync(p.text));
      } catch (e) {
        p.reject(e);
      }
      if (nowMs() - started >= FALLBACK_BATCH_MS) break;
    }
    if (i < items.length) window.setTimeout(runBatch, 0);
  };
  runBatch();
}

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
      if (pending.size > 0) armStallTimer();
      else clearStallTimer();
      p.resolve(e.data.raw);
    };
    w.onerror = () => {
      // Worker 异常：标记不可用并回退同步解析
      clearStallTimer();
      worker = null;
      try {
        w.terminate();
      } catch {}
      drainPendingSync();
    };
    worker = w;
  } catch {
    worker = null;
  }
  return worker;
}

/**
 * 渲染 Markdown：优先 Web Worker 离屏解析，主线程只做 sanitize；
 * Worker 不可用/停滞/出错时回退主线程同步解析。返回清洗后的 HTML。
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
    pending.set(id, {
      text,
      resolve: (raw) => resolve(sanitizeMarkdown(raw)),
      reject,
    });
    armStallTimer();
    try {
      w.postMessage({ id, text });
    } catch {
      pending.delete(id);
      if (pending.size > 0) armStallTimer();
      else clearStallTimer();
      runSync();
    }
  });
}
