/**
 * 从计划 Markdown 中提取首个标题行作为卡片标题，其余内容作为正文。
 * 标题为 ATX 风格（#~###### 起始，允许至多 3 个前导空格）；标题行从正文中
 * 移除避免重复展示，正文保留其后内容（含其它标题）。
 * 无标题行时标题回退 "计划"，正文为原文。
 */
export function splitPlanTitle(planText: string): {
  title: string;
  body: string;
} {
  const text = planText.trimStart();
  if (!text) return { title: "计划", body: "" };
  const lines = text.split("\n");
  const idx = lines.findIndex((line) => /^\s{0,3}#{1,6}(?:\s+|$)/.test(line));
  if (idx < 0) return { title: "计划", body: text };
  const title = lines[idx].replace(/^\s{0,3}#{1,6}\s*/, "").trim() || "计划";
  lines.splice(idx, 1);
  return { title, body: lines.join("\n").trimStart() };
}
