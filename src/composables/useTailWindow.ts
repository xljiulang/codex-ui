import { onScopeDispose, ref, watch, type Ref } from "vue";
import {
  utf8ByteLength,
  type TextTruncation,
  type TextWindowLimits,
} from "../lib/textWindow";

export interface TailWindow {
  ref: Ref<string>;
  truncated: Ref<boolean>;
  truncation: Ref<TextTruncation>;
  /** 立即处理当前源值（流式结束时用于抹平最后一帧）。 */
  flush: () => void;
}

function countNewlines(s: string): number {
  let n = 0;
  for (let i = 0; i < s.length; i++) {
    if (s.charCodeAt(i) === 10) n++;
  }
  return n;
}

const TAIL_WINDOW_FLUSH_MS = 80;

/**
 * 增量维护源字符串的末尾 maxLines 行窗口（语义等同 split("\n").slice(-maxLines).join("\n")）：
 * 每段 delta 只处理新增部分，超限时摊销丢弃头部整行，避免对完整输出反复 split。
 * resetSignal 变化（如条目对象被整体替换）时重置增量状态并重放当前源。
 */
export function useTailWindow(
  source: Ref<string>,
  limits: number | TextWindowLimits,
  resetSignal?: () => unknown,
): TailWindow {
  const maxLines =
    typeof limits === "number" ? limits : Math.max(1, limits.maxLines);
  const maxBytes = typeof limits === "number" ? undefined : limits.maxBytes;
  const out = ref("");
  const truncated = ref(false);
  const truncation = ref<TextTruncation>("none");
  let processedLen = 0; // 已处理的源长度
  let tail = "";
  let tailNewlines = 0; // 窗口内换行数（窗口行数 = tailNewlines + 1）
  let tailBytes = 0;
  let timer: number | undefined;

  function noteTruncation(reason: Exclude<TextTruncation, "none">) {
    if (truncation.value === "none") truncation.value = reason;
    else if (truncation.value !== reason) truncation.value = "both";
    truncated.value = true;
  }

  function clearTimer() {
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timer = undefined;
    }
  }

  function reset() {
    clearTimer();
    processedLen = 0;
    tail = "";
    tailNewlines = 0;
    tailBytes = 0;
    truncated.value = false;
    truncation.value = "none";
    out.value = "";
  }

  function trimHeadToLimits() {
    if (maxBytes !== undefined && tailBytes > maxBytes) {
      const excess = tailBytes - maxBytes;
      let cut = 0;
      let removedBytes = 0;
      while (cut < tail.length && removedBytes < excess) {
        const cp = tail.codePointAt(cut) ?? 0;
        removedBytes += cp <= 0x7f ? 1 : cp <= 0x7ff ? 2 : cp <= 0xffff ? 3 : 4;
        cut += cp > 0xffff ? 2 : 1;
      }
      const removed = tail.slice(0, cut);
      tail = tail.slice(cut);
      tailBytes -= removedBytes;
      tailNewlines -= countNewlines(removed);
      noteTruncation("bytes");
    }

    while (tailNewlines + 1 > maxLines) {
      const at = tail.indexOf("\n");
      if (at < 0) break;
      const removed = tail.slice(0, at + 1);
      tail = tail.slice(at + 1);
      tailBytes -= utf8ByteLength(removed);
      tailNewlines--;
      noteTruncation("lines");
    }
  }

  function sync(full: string) {
    if (full.length < processedLen) {
      // 条目整体替换/回退：重置增量状态后重放
      reset();
    }
    const added = full.slice(processedLen);
    processedLen = full.length;
    if (!added) return;
    tail += added;
    const nl = countNewlines(added);
    tailNewlines += nl;
    tailBytes += utf8ByteLength(added);
    trimHeadToLimits();
    out.value = tail;
  }

  function schedule() {
    if (timer !== undefined) return;
    timer = window.setTimeout(() => {
      timer = undefined;
      sync(source.value);
    }, TAIL_WINDOW_FLUSH_MS);
  }

  function flush() {
    clearTimer();
    sync(source.value);
  }

  // 初始值同步呈现；后续流式更新合并到 80ms 窗口，避免每个 delta 都复制大字符串。
  sync(source.value);
  watch(source, schedule, { flush: "sync" });
  if (resetSignal) {
    watch(
      resetSignal,
      () => {
        reset();
        sync(source.value);
      },
      { flush: "sync" },
    );
  }
  onScopeDispose(clearTimer);
  return { ref: out, truncated, truncation, flush };
}
