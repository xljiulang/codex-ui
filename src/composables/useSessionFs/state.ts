import { computed, reactive, ref } from "vue";
import {
  flattenResourceTree,
  isPathUnderRoot,
  type FsEntry,
  type ResourceRow,
  type SearchSnippet,
} from "../../lib/sessionFs";

export const rootEntry = ref<FsEntry | null>(null);
export const rootError = ref("");
export const loadingRoot = ref(false);
/** 目录路径 → 已加载的直接子项（懒加载缓存） */
export const childrenByPath = reactive<Record<string, FsEntry[]>>({});
/** 展开中的目录路径集合（含根） */
export const expanded = reactive(new Set<string>());
export const loadingByPath = reactive<Record<string, boolean>>({});

export const searchTerm = ref("");
export const searchResults = ref<FsEntry[]>([]);
/** 内容命中摘要：键为 normalizePathKey(entry.path) */
export const searchSnippets = ref<Record<string, SearchSnippet>>({});
export const searching = ref(false);
export const selectedPath = ref("");
/**
 * 文件类型图标缓存：键为 `ext:<小写扩展名>`（如 ext:.rs）或无扩展名文件的
 * `file:<relPath>`；值为 PNG data URI，null 表示取不到（不再重试）。
 * 目录不在范围，继续使用内置 SVG 文件夹图标。
 */
export const iconCache = reactive(new Map<string, string | null>());

export const searchActive = computed(() => searchTerm.value.trim().length > 0);

export const treeRows = computed<ResourceRow[]>(() => {
  if (!rootEntry.value) return [];
  return flattenResourceTree(rootEntry.value, childrenByPath, expanded);
});

/** 清空全部树/搜索状态（根切换与测试重置共用） */
export function resetTree() {
  for (const k of Object.keys(childrenByPath)) delete childrenByPath[k];
  for (const k of Object.keys(loadingByPath)) delete loadingByPath[k];
  expanded.clear();
  rootEntry.value = null;
  rootError.value = "";
  searchResults.value = [];
  searchSnippets.value = {};
  selectedPath.value = "";
  iconCache.clear();
}

/** 根切换成功后：清理旧根缓存的 childrenByPath/expanded/selectedPath，
 * 避免 watcher 事件触发 refreshAll 时误加载旧根路径 */
export function pruneTreeToRoot(root: string) {
  for (const k of Object.keys(childrenByPath)) {
    if (!isPathUnderRoot(root, k)) delete childrenByPath[k];
  }
  for (const k of [...expanded]) {
    if (!isPathUnderRoot(root, k)) expanded.delete(k);
  }
  if (selectedPath.value && !isPathUnderRoot(root, selectedPath.value)) {
    selectedPath.value = "";
  }
}

/** 目录已不可列（被删除/改名等）时，从缓存中剪枝：移除自身及所有后代目录键并取消展开，
 *  避免 refreshAll 每次 fs 事件都重列一个已消失的目录而反复弹「无法访问」。 */
export function pruneDeadDir(path: string) {
  const base = path.replace(/[\\/]+$/, "");
  const isSelfOrDesc = (k: string) =>
    k === base || k.startsWith(base + "\\") || k.startsWith(base + "/");
  for (const k of Object.keys(childrenByPath)) {
    if (isSelfOrDesc(k)) delete childrenByPath[k];
  }
  for (const k of Object.keys(loadingByPath)) {
    if (isSelfOrDesc(k)) delete loadingByPath[k];
  }
  for (const k of [...expanded]) {
    if (isSelfOrDesc(k)) expanded.delete(k);
  }
}
