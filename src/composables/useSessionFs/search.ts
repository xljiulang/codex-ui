import { invoke } from "@tauri-apps/api/core";
import { setToast, toastError, workspace } from "../useCodex";
import { debounce } from "../../lib/debounce";
import {
  mergeSearchResults,
  type FsEntry,
  type RgSearchResult,
  type SearchSnippet,
} from "../../lib/sessionFs";
import { normalizePathKey } from "../../lib/path";
import {
  searchResults,
  searchSnippets,
  searching,
  searchTerm,
} from "./state";

const SEARCH_LIMIT = 200;
const RG_SEARCH_LIMIT = 100;
/** 搜索序号：结果晚到（旧序号）时丢弃，避免慢响应覆盖新结果 */
let searchSeq = 0;

export const debouncedSearch = debounce(() => void runSearchNow(), 300);

export function onSearchInput() {
  debouncedSearch.run();
}

/** 搜索命中行摘要（按规范化路径查） */
export function searchSnippet(entry: FsEntry): SearchSnippet | null {
  return searchSnippets.value[normalizePathKey(entry.path)] ?? null;
}

export async function runSearchNow() {
  const q = searchTerm.value.trim();
  if (!q) {
    searchSeq++;
    searchResults.value = [];
    searchSnippets.value = {};
    searching.value = false;
    return;
  }
  const root = workspace.value;
  if (!root) return;
  const seq = ++searchSeq;
  searching.value = true;
  let nameHits: FsEntry[] = [];
  let contentHits: RgSearchResult["hits"] = [];
  const apply = () => {
    if (seq !== searchSeq) return;
    const merged = mergeSearchResults(nameHits, contentHits);
    searchResults.value = merged.results;
    searchSnippets.value = merged.snippets;
  };
  // 文件名搜索立即发起；rg 探测与内容搜索并行进行，互不阻塞
  const nameTask = invoke<FsEntry[]>("session_fs_search", {
    workspace: root,
    query: q,
    limit: SEARCH_LIMIT,
  })
    .then((res) => {
      if (seq !== searchSeq) return;
      nameHits = res ?? [];
      apply();
    })
    .catch((e) => {
      if (seq !== searchSeq) return;
      nameHits = [];
      apply();
      setToast(toastError(e));
    });
  const contentTask = invoke<{ available?: boolean }>("session_fs_rg_status")
    .then((status) => {
      if (seq !== searchSeq || !status?.available) return null;
      return invoke<RgSearchResult>("session_fs_search_rg", {
        workspace: root,
        query: q,
        limit: RG_SEARCH_LIMIT,
      });
    })
    .then((res) => {
      if (seq !== searchSeq || !res?.available) return;
      contentHits = res.hits ?? [];
      apply();
    })
    .catch(() => {
      // rg 探测/搜索失败静默回退：文件名结果不受影响
    });
  await Promise.allSettled([nameTask, contentTask]);
  if (seq === searchSeq) searching.value = false;
}

export function clearSearch() {
  searchTerm.value = "";
  debouncedSearch.cancel();
  searchSeq++;
  searchResults.value = [];
  searchSnippets.value = {};
  searching.value = false;
}

/** 测试重置：清空搜索状态（防抖取消 + 结果失效） */
export function resetSearchState() {
  searchTerm.value = "";
  searching.value = false;
  debouncedSearch.cancel();
  searchSeq++;
  searchResults.value = [];
  searchSnippets.value = {};
}
