// useCodex 拆分模块：设置/服务/模型/插件/技能（原 useCodex.ts 的一部分，纯移动，行为不变）
import { invoke } from "@tauri-apps/api/core";
import { PERMISSION_MODES } from "../../lib/permissions";
import type { AppSettings, ServerStatus } from "../../lib/types";
import { applyTheme } from "../useTheme";
import { store } from "./store";
import { defaultSettings, type ModelInfo, type PluginItem, type SkillItem } from "./types";


/** 当前会话已不存在（被删除等）时重置回新对话，避免继续发送一直报错 */
export function resetToNewChat() {
  store.currentThreadId = null;
  store.currentThreadName = "";
  store.currentThreadOrigin = null;
  store.currentThreadWorkspace = null;
  store.resumedThreadId = null;
  store.currentTurnId = null;
  store.turnActive = false;
  store.turnInterrupted = false;
  store.goalText = null;
  store.goalStatus = null;
  store.goalArmed = false;
}


export async function loadSettings() {
  try {
    const s = await invoke<AppSettings>("settings_get");
    store.settings = { ...defaultSettings(), ...s };
  } catch {
    store.settings = defaultSettings();
  }
  // 持久化值非法时回退默认；默认权限作为权限模式的启动初始值（运行期切换不写回配置）
  if (
    !PERMISSION_MODES.some((m) => m.id === store.settings.default_permission)
  ) {
    store.settings.default_permission = "ask-for-approval";
  }
  if (!["enabled", "disabled"].includes(store.settings.memory_mode)) {
    store.settings.memory_mode = "disabled";
  }
  store.permissionMode = store.settings.default_permission;
  applyTheme(store.settings.theme);
}


export async function saveSettings(patch: Partial<AppSettings>) {
  store.settings = { ...store.settings, ...patch };
  await invoke("settings_set", { settings: store.settings });
  applyTheme(store.settings.theme);
}


export async function refreshServer() {
  const s = await invoke<ServerStatus>("server_status");
  store.server = { ...store.server, ...s };
}


/** 拉取可用模型列表（幂等），供模型菜单与输入区按钮共用 */
export async function loadModels(force = false) {
  if (store.modelsLoaded && !force) return;
  try {
    const res = await invoke<{ data: ModelInfo[] }>("codex_rpc", {
      method: "model/list",
      params: {},
    });
    store.models = (res.data ?? []).filter((m) => !m.hidden);
    store.modelsLoaded = true;
  } catch {
    // 模型列表不可用时保持空，UI 回退
  }
}


/** 确保指定对话的插件缓存已加载（对话级缓存：已加载直接返回，不回退 skills/list） */
export async function ensureThreadPlugins(threadId: string) {
  if (store.threadPlugins[threadId]?.loaded) return;
  store.threadPlugins[threadId] = { plugins: [], loaded: false };
  try {
    const res = await invoke<{
      marketplaces?: {
        plugins?: {
          id?: string;
          name: string;
          installed?: boolean;
          enabled?: boolean;
          source?: { path?: string };
          interface?: {
            displayName?: string;
            shortDescription?: string;
            longDescription?: string;
            composerIcon?: string;
            composerIconUrl?: string | null;
            brandColor?: string;
          };
        }[];
      }[];
    }>("codex_rpc", {
      method: "plugin/list",
      params: {},
    });
    const list: PluginItem[] = [];
    const seen = new Set<string>();
    for (const mp of res?.marketplaces ?? []) {
      for (const p of mp.plugins ?? []) {
        if (p.installed === false || p.enabled === false) continue;
        const id = p.id ?? p.name;
        if (seen.has(id)) continue;
        seen.add(id);
        list.push({
          id,
          name: p.name,
          displayName: p.interface?.displayName ?? p.name,
          description:
            p.interface?.shortDescription ?? p.interface?.longDescription ?? "",
          path: p.source?.path ?? "",
          iconPath: p.interface?.composerIcon ?? "",
          iconUrl: p.interface?.composerIconUrl ?? "",
          brandColor: p.interface?.brandColor ?? "",
        });
      }
    }
    store.threadPlugins[threadId] = { plugins: list, loaded: true };
  } catch {
    // 插件列表不可用时保持空，不回退 skills/list
    store.threadPlugins[threadId] = { plugins: [], loaded: false };
  }
}


/** 拉取技能列表（全局缓存，幂等），供 $ 菜单与回显悬浮提示使用 */
export async function ensureSkills(force = false) {
  if (store.skillsLoaded && !force) return;
  try {
    const res = await invoke<{
      data?: {
        skills?: (SkillItem & {
          description?: string;
          interface?: { shortDescription?: string };
        })[];
      }[];
    }>("codex_rpc", { method: "skills/list", params: {} });
    const list = (res?.data ?? [])
      .flatMap((d) => d.skills ?? [])
      .filter((s) => (s as { enabled?: boolean }).enabled !== false)
      .map<SkillItem>((s) => ({
        name: s.name,
        key: s.name,
        path: s.path ?? "",
        desc: s.description ?? s.interface?.shortDescription ?? s.desc ?? "",
        shortDesc:
          s.interface?.shortDescription ?? s.description ?? s.desc ?? "",
      }));
    store.skills = list;
    store.skillsLoaded = true;
  } catch {
    // 技能列表不可用时保持空
    store.skills = [];
  }
}


/** 解析模型的显示名：指定模型优先，否则用默认模型 */
export function modelDisplayName(model: string | null): string {
  if (model) {
    const m = store.models.find((x) => x.model === model);
    return m?.displayName || model;
  }
  const def = store.models.find((x) => x.isDefault);
  return def?.displayName || "默认模型";
}


/** 当前生效模型 id：显式选择 → 会话已知模型 → 默认模型 → 列表首个 → 空 */
export function currentModelId(): string {
  if (store.model) return store.model;
  if (store.currentModel) return store.currentModel;
  const def = store.models.find((m) => m.isDefault);
  if (def) return def.model;
  return store.models[0]?.model ?? "";
}


/** 当前生效的推理强度：显式值优先，否则用默认模型的默认强度 */
export function effectiveEffort(): string {
  if (store.effort) return store.effort;
  const m =
    store.models.find((x) => x.model === store.model) ??
    store.models.find((x) => x.isDefault);
  return m?.defaultReasoningEffort ?? "";
}
