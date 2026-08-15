import { computed, reactive, ref } from "vue";
import {
  flattenResourceTree,
  type FsEntry,
  type ResourceRow,
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
export const searching = ref(false);
export const selectedPath = ref("");
/** 内部复制记录：粘贴时优先使用，为空回退系统剪贴板 */
export const copyBuffer = ref<string[]>([]);
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

/** 清空全部树/搜索/剪贴板状态（根切换与测试重置共用） */
export function resetTree() {
  for (const k of Object.keys(childrenByPath)) delete childrenByPath[k];
  for (const k of Object.keys(loadingByPath)) delete loadingByPath[k];
  expanded.clear();
  rootEntry.value = null;
  rootError.value = "";
  searchResults.value = [];
  selectedPath.value = "";
  copyBuffer.value = [];
  iconCache.clear();
}
