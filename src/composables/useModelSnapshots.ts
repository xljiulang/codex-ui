// 模型快照：标题栏下拉的列表/新建/还原/删除/打开。
// 后端在 CODEX_HOME/codex-ui/<名称>.json 下直接管理模型配置快照；
// 保存/还原不调用 app-server，还原后需重启 codex-ui 生效。
import { invoke } from "@tauri-apps/api/core";
import { askConfirm, setToast } from "./useCodex";
import { openPathInAppOrReveal } from "./usePathOpen";

/** 列出全部模型快照名（来自 codex-home/codex-ui 下的 .json 文件，按名排序）。 */
export async function listModelSnapshots(): Promise<string[]> {
  return invoke<string[]>("model_snapshots_list");
}

/** 新建/覆盖模型快照：直接读取当前 config.toml 与模型目录文件写入快照。 */
export async function createModelSnapshot(name: string): Promise<void> {
  const n = name.trim();
  await invoke("model_snapshots_save", { name: n });
  setToast(`已创建模型快照「${n}」（可能含 API Key，请谨慎分享）`);
}

/** 还原模型快照：直接写 config.toml 与模型目录文件，重启 codex-ui 后生效。 */
export async function applyModelSnapshot(name: string): Promise<void> {
  await invoke("model_snapshots_apply", { name });
  setToast(`已应用模型快照「${name}」，重启 codex-ui 后生效`);
}

/** 删除模型快照：先全局确认，再删除对应 .json 文件。 */
export async function deleteModelSnapshot(name: string): Promise<void> {
  const ok = await askConfirm({
    title: "删除模型快照",
    message: `确定删除模型快照「${name}」吗？此操作不可撤销。`,
    confirmLabel: "删除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  await invoke("model_snapshots_delete", { name });
  setToast(`已删除模型快照「${name}」`);
}

/** 打开模型快照 JSON 文件。 */
export async function openModelSnapshot(name: string): Promise<void> {
  const path = await invoke<string>("model_snapshots_open", { name });
  await openPathInAppOrReveal(path);
}
