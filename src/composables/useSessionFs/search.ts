import { invoke } from "@tauri-apps/api/core";
import { setToast, toastError, workspace } from "../useCodex";
import { debounce } from "../../lib/debounce";
import type { FsEntry } from "../../lib/sessionFs";
import { searchResults, searching, searchTerm } from "./state";

const SEARCH_LIMIT = 200;
/** 搜索序号：结果晚到（旧序号）时丢弃，避免慢响应覆盖新结果 */
let searchSeq = 0;

export const debouncedSearch = debounce(() => void runSearchNow(), 300);

export function onSearchInput() {
  debouncedSearch.run();
}

export async function runSearchNow() {
  const q = searchTerm.value.trim();
  if (!q) {
    searchSeq++;
    searchResults.value = [];
    searching.value = false;
    return;
  }
  const root = workspace.value;
  if (!root) return;
  const seq = ++searchSeq;
  searching.value = true;
  try {
    const res = await invoke<FsEntry[]>("session_fs_search", {
      workspace: root,
      query: q,
      limit: SEARCH_LIMIT,
    });
    if (seq === searchSeq) searchResults.value = res;
  } catch (e) {
    if (seq === searchSeq) {
      searchResults.value = [];
      setToast(toastError(e));
    }
  } finally {
    if (seq === searchSeq) searching.value = false;
  }
}

export function clearSearch() {
  searchTerm.value = "";
  debouncedSearch.cancel();
  searchSeq++;
  searchResults.value = [];
  searching.value = false;
}

/** 测试重置：清空搜索状态（防抖取消 + 结果失效） */
export function resetSearchState() {
  searchTerm.value = "";
  searching.value = false;
  debouncedSearch.cancel();
  searchSeq++;
}
