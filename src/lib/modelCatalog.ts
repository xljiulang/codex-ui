function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

/**
 * 按选中的模型 ID 过滤已生成的模型目录，并按原顺序重排 priority。
 * 模型目录由后端生成，结构非法时抛出错误供调用方提示。
 */
export function filterCatalogByModelIds(
  catalogText: string,
  selectedIds: Iterable<string>,
): string {
  const selected = new Set(selectedIds);
  const parsed: unknown = JSON.parse(catalogText);
  if (!isRecord(parsed)) {
    throw new Error("模型目录不是 JSON 对象");
  }
  if (!Array.isArray(parsed.models)) {
    throw new Error("模型目录缺少 models 数组");
  }

  const models = parsed.models
    .filter(isRecord)
    .filter((entry) => {
      const id =
        typeof entry.slug === "string"
          ? entry.slug
          : typeof entry.id === "string"
            ? entry.id
            : "";
      return selected.has(id);
    })
    .map((entry, index) => ({ ...entry, priority: index + 1 }));

  return JSON.stringify({ ...parsed, models }, null, 2);
}
