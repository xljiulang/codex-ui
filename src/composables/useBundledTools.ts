// 捆绑 CLI 工具：检测 codex 生效 PATH 上的 ast-grep / fd / rg，
// 供默认协作模式的 developer_instructions 注入使用说明。
import { invoke } from "@tauri-apps/api/core";
import { ref } from "vue";

/** 当前 codex 生效 PATH 上检测到的捆绑 CLI 基名列表（空 = 无可用工具，不注入） */
export const availableBundledTools = ref<string[]>([]);

/** 各工具的使用说明（示例用纯文本，避免模板反引号嵌套）；未知工具回退基名 */
const TOOL_HINTS: Record<string, string> = {
  rg: "rg：文本搜索（等价递归 grep，速度更快）。例：rg -n 关键字 .",
  fd: "fd：按名快速查找文件/目录，默认尊重 .gitignore。例：fd 文件名 .",
  "ast-grep":
    "ast-grep / sg：基于 AST 的结构化源码搜索与改写。例：sg --pattern '...'",
};

/** 拉取当前可用的捆绑 CLI 工具（失败置空，避免误注入） */
export async function loadBundledTools(): Promise<void> {
  try {
    availableBundledTools.value = await invoke<string[]>("cli_tools_available");
  } catch {
    availableBundledTools.value = [];
  }
}

/** 默认协作模式的开发者指令；无可用工具时返回 null（不注入） */
export function bundledToolsDeveloperInstructions(): string | null {
  const tools = availableBundledTools.value;
  if (tools.length === 0) return null;
  const lines = tools.map((t) => `- ${TOOL_HINTS[t] ?? t}`).join("\n");
  return `# Collaboration Mode: Default

你处于默认协作模式，可正常读取与修改工作区文件。

## 本应用附带的 CLI 工具（已加入 codex 进程 PATH，可直接调用）
${lines}`;
}
