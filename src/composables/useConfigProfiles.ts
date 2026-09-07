// 配置快照：标题栏「配置」下拉的列表/新建/应用/删除。
// 后端在 CODEX_HOME/codex-ui/<配置名> 下管理 config.toml + models.json 快照，
// 前端仅负责调用命令、应用后触发 codex 热重载。快照间不再记录/维护「当前激活」关系。
import { invoke } from "@tauri-apps/api/core";
import { askConfirm, setToast } from "./useCodex";
import { openPathInAppOrReveal } from "./usePathOpen";

/** config/read 返回的配置层（取子集，与服务端 schema 对齐） */
interface RawConfigReadResponse {
  config?: Record<string, unknown>;
}

/** 列出全部配置快照名（来自 codex-home/codex-ui 下的文件夹，按名排序）。 */
export async function listConfigProfiles(): Promise<string[]> {
  return invoke<string[]>("config_profiles_list");
}

/** 新建/覆盖配置快照：把当前 codex-home 的 config.toml 与 model_catalog 内容快照到该配置目录。 */
export async function createConfigProfile(name: string): Promise<void> {
  await invoke("config_profiles_save", { name });
}

/** 还原配置快照：覆盖 codex-home 文件并触发 codex 热重载。 */
export async function applyConfigProfile(name: string): Promise<void> {
  await invoke("config_profiles_apply", { name });
  await reloadUserConfig();
  setToast(`已应用配置快照「${name}」，重启 codex-ui 后生效`);
}

/** 删除配置：先全局确认，再删除对应快照目录。 */
export async function deleteConfigProfile(name: string): Promise<void> {
  const ok = await askConfirm({
    title: "删除配置",
    message: `确定删除配置快照「${name}」吗？此操作不可撤销。`,
    confirmLabel: "删除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  await invoke("config_profiles_delete", { name });
  setToast(`已删除配置「${name}」`);
}

/** 打开配置快照：把该快照目录下的 config.toml 与模型目录文件在编辑器打开。 */
export async function openConfigProfile(name: string): Promise<void> {
  const files = await invoke<string[]>("config_profiles_open", { name });
  for (const f of files) {
    await openPathInAppOrReveal(f);
  }
}

/** 让 codex 重读已覆盖到磁盘的 config.toml：优先空编辑触发 reloadUserConfig，
 *  若 codex 报错则回退为以当前 model 值做一次无变更写回。 */
async function reloadUserConfig(): Promise<void> {
  try {
    await invoke("codex_rpc", {
      method: "config/batchWrite",
      params: { edits: [], reloadUserConfig: true },
    });
  } catch {
    const res = await invoke<RawConfigReadResponse>("codex_rpc", {
      method: "config/read",
      params: { includeLayers: true },
    });
    const model = res?.config?.model;
    if (typeof model === "string") {
      await invoke("codex_rpc", {
        method: "config/batchWrite",
        params: {
          edits: [{ keyPath: "model", value: model, mergeStrategy: "replace" }],
          reloadUserConfig: true,
        },
      });
    }
  }
}
