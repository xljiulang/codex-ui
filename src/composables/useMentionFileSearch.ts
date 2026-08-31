import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { setToast, toastError, workspace } from "./useCodex";
import { debounce } from "../lib/debounce";
import type { FuzzyFileResult } from "../lib/mention";

/** @ 提及的文件模糊搜索（防抖 250ms、序号失效、结果上限 50 条） */
export function useMentionFileSearch() {
  const fileResults = ref<FuzzyFileResult[]>([]);
  const searchingFiles = ref(false);
  let searchSeq = 0;

  const debouncedFileSearch = debounce(
    (token: string) => void runFileSearch(token),
    250,
  );

  /** 清空搜索状态；invalidate=true 时使进行中的异步结果失效 */
  function resetFileSearch(invalidate = false) {
    fileResults.value = [];
    searchingFiles.value = false;
    debouncedFileSearch.cancel();
    if (invalidate) searchSeq++;
  }

  function scheduleFileSearch(token: string) {
    if (!token) {
      resetFileSearch(true);
      return;
    }
    debouncedFileSearch.run(token);
  }

  async function runFileSearch(token: string) {
    const seq = ++searchSeq;
    // 跟随全局工作区（与资源/Git 面板同源）；无确定工作区时不搜索
    const root = workspace.value;
    if (!root) {
      searchingFiles.value = false;
      return;
    }
    searchingFiles.value = true;
    try {
      const res = await invoke<{ files?: FuzzyFileResult[] }>("codex_rpc", {
        method: "fuzzyFileSearch",
        params: { query: token, roots: [root], cancellationToken: null },
      });
      if (seq !== searchSeq) return;
      fileResults.value = (res.files ?? []).slice(0, 50);
    } catch (e) {
      if (seq === searchSeq) {
        fileResults.value = [];
        setToast(toastError(e));
      }
    } finally {
      if (seq === searchSeq) searchingFiles.value = false;
    }
  }

  return {
    fileResults,
    searchingFiles,
    scheduleFileSearch,
    resetFileSearch,
    cancelFileSearch: () => debouncedFileSearch.cancel(),
  };
}
