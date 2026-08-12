import { ref, watch, type Ref } from "vue";

export interface TailWindow {
  ref: Ref<string>;
  truncated: Ref<boolean>;
}

function countNewlines(s: string): number {
  let n = 0;
  for (let i = 0; i < s.length; i++) {
    if (s.charCodeAt(i) === 10) n++;
  }
  return n;
}

/**
 * 增量维护源字符串的末尾 maxLines 行窗口（语义等同 split("\n").slice(-maxLines).join("\n")）：
 * 每段 delta 只处理新增部分，超限时摊销丢弃头部整行，避免对完整输出反复 split。
 * resetSignal 变化（如条目对象被整体替换）时重置增量状态并重放当前源。
 */
export function useTailWindow(
  source: Ref<string>,
  maxLines: number,
  resetSignal?: () => unknown,
): TailWindow {
  const out = ref("");
  const truncated = ref(false);
  let processedLen = 0; // 已处理的源长度
  let totalNewlines = 0; // 源累计换行数（截断判定）
  let tail = "";
  let tailNewlines = 0; // 窗口内换行数（窗口行数 = tailNewlines + 1）

  function sync(full: string) {
    if (full.length < processedLen) {
      // 条目整体替换/回退：重置增量状态后重放
      processedLen = 0;
      totalNewlines = 0;
      tail = "";
      tailNewlines = 0;
      truncated.value = false;
      out.value = "";
    }
    const added = full.slice(processedLen);
    processedLen = full.length;
    if (!added) return;
    tail += added;
    const nl = countNewlines(added);
    totalNewlines += nl;
    tailNewlines += nl;
    while (tailNewlines + 1 > maxLines) {
      const at = tail.indexOf("\n");
      if (at < 0) break;
      tail = tail.slice(at + 1);
      tailNewlines--;
    }
    truncated.value = totalNewlines + 1 > maxLines;
    out.value = tail;
  }

  watch(source, (v) => sync(v), { immediate: true, flush: "sync" });
  if (resetSignal) {
    watch(
      resetSignal,
      () => {
        processedLen = 0;
        totalNewlines = 0;
        tail = "";
        tailNewlines = 0;
        truncated.value = false;
        out.value = "";
        sync(source.value);
      },
      { flush: "sync" },
    );
  }
  return { ref: out, truncated };
}
