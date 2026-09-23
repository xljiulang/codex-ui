// useCodex 拆分模块：设置/服务/模型/插件/技能（原 useCodex.ts 的一部分，纯移动，行为不变）
import { invoke } from "@tauri-apps/api/core";
import { PERMISSION_MODES } from "../../lib/permissions";
import type { AppSettings, ServerStatus } from "../../lib/types";
import { applyGlassEffect, applyTheme } from "../useTheme";
import { activeSessionTab, sessionTabTitle } from "./sessionState";
import { store } from "./store";
import {
  defaultSettings,
  type ModelInfo,
  type PluginItem,
  type SessionTab,
  type SkillItem,
} from "./types";

/** 当前会话已不存在（被删除等）时重置回新对话，避免继续发送一直报错 */
export function resetToNewSession() {
  const tab = activeSessionTab();
  if (!tab) return;
  tab.threadId = null;
  tab.name = "";
  tab.origin = null;
  tab.workspace = null;
  tab.resumedThreadId = null;
  tab.turnActive = false;
  tab.currentTurnId = null;
  tab.turnInterrupted = false;
  tab.goalText = null;
  tab.goalStatus = null;
  tab.goalArmed = false;
  tab.plugins = { plugins: [], loaded: false };
  tab.skills = { skills: [], loaded: false };
  tab.planPrompt = null;
  tab.followupQueue = [];
  tab.title = sessionTabTitle(tab);
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
  if (!["cmd", "powershell"].includes(store.settings.terminal_shell)) {
    store.settings.terminal_shell = "cmd";
  }
  if (!Array.isArray(store.settings.dynamic_tools_disabled)) {
    store.settings.dynamic_tools_disabled = [];
  }
  const tab = activeSessionTab();
  if (tab) tab.permissionMode = store.settings.default_permission;
  applyTheme(store.settings.theme);
  applyGlassEffect(store.settings.glass_effect);
}

export async function saveSettings(patch: Partial<AppSettings>) {
  store.settings = { ...store.settings, ...patch };
  await invoke("settings_set", { settings: store.settings });
  applyTheme(store.settings.theme);
  applyGlassEffect(store.settings.glass_effect);
}

export async function refreshServer() {
  const s = await invoke<ServerStatus>("server_status");
  store.server = { ...store.server, ...s };
}

/** app-server 协议 ReasoningEffort 合法值（与设置页档位列表一致） */
const REASONING_EFFORTS = new Set([
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
]);

/** config/read 返回结构中本模块用到的字段 */
interface RawConfigReadResponse {
  config?: {
    model?: unknown;
    model_reasoning_effort?: unknown;
  } | null;
}

/** 活动会话 cwd 的有效配置默认值（model/model_reasoning_effort）；null 表示读取失败 */
interface EffectiveDefaults {
  model: string;
  reasoningEffort: string;
}

/** 调用 config/read（cwd 为空则省略，返回线程无关配置）；响应形状非法按失败处理 */
async function invokeConfigRead(
  cwd: string,
): Promise<RawConfigReadResponse | null> {
  const params: Record<string, unknown> = { includeLayers: true };
  if (cwd) params.cwd = cwd;
  try {
    const res = await invoke<RawConfigReadResponse>("codex_rpc", {
      method: "config/read",
      params,
    });
    if (
      !res ||
      typeof res !== "object" ||
      !res.config ||
      typeof res.config !== "object"
    ) {
      return null;
    }
    return res;
  } catch {
    return null;
  }
}

/**
 * 读取活动会话 cwd 的有效配置默认 model/强度：与 thread/start 使用同一 cwd
 * （含项目层/托管覆盖）。带 cwd 读取失败时重试一次线程无关配置；仍失败返回 null。
 */
async function readEffectiveDefaults(
  cwd: string,
): Promise<EffectiveDefaults | null> {
  let res = await invokeConfigRead(cwd);
  if (!res && cwd) res = await invokeConfigRead("");
  if (!res) return null;
  const rawModel = res.config?.model;
  const rawEffort = res.config?.model_reasoning_effort;
  const effort = typeof rawEffort === "string" ? rawEffort.trim() : "";
  return {
    model: typeof rawModel === "string" ? rawModel.trim() : "",
    reasoningEffort: REASONING_EFFORTS.has(effort) ? effort : "",
  };
}

/** 拉取 model/list 目录（过滤 hidden）；失败或响应形状非法返回 null */
async function fetchModelCatalog(): Promise<ModelInfo[] | null> {
  try {
    const res = await invoke<{ data?: unknown }>("codex_rpc", {
      method: "model/list",
      params: {},
    });
    if (!res || !Array.isArray(res.data)) return null;
    return (res.data as ModelInfo[]).filter((m) => !m.hidden);
  } catch {
    return null;
  }
}

/**
 * 按有效配置合成 UI 模型列表：
 * - 配置 model 命中目录：原顺序不变，仅命中行 isDefault；
 * - 配置 model 未命中（含被 hidden 过滤）：置顶合成项并标记默认；
 * - 配置 model 为空：不插合成项，沿用服务端 isDefault（缺失则首项）；
 * - 配置读取失败（null）：同"为空"分支沿用服务端 isDefault；
 * - 默认项强度：config model_reasoning_effort 非空时覆盖该项 defaultReasoningEffort；
 *   目录没声明档位（supportedReasoningEfforts 为空 = 未知）的条目也沿用该值——
 *   否则切到这类模型时强度会回落成 none，整轮不请求推理（Zen 的 big-pickle 实测）。
 *   声明了档位的模型（除配置模型那一项）不受影响。
 */
function composeModels(
  catalog: ModelInfo[],
  defaults: EffectiveDefaults | null,
): ModelInfo[] {
  const configEffort = defaults?.reasoningEffort ?? "";
  const applyEffort = (m: ModelInfo, isConfigModel: boolean): ModelInfo => {
    if (!configEffort) return m;
    if (!isConfigModel && m.supportedReasoningEfforts.length > 0) return m;
    return { ...m, defaultReasoningEffort: configEffort };
  };
  const list = catalog.map((m) => ({ ...m }));
  if (defaults === null || !defaults.model) {
    if (!list.some((m) => m.isDefault) && list.length) list[0].isDefault = true;
    return list.map((m) => applyEffort(m, m.isDefault));
  }
  const configModel = defaults.model;
  const idx = list.findIndex((m) => m.model === configModel);
  if (idx >= 0) {
    return list.map((m, i) =>
      applyEffort({ ...m, isDefault: i === idx }, i === idx),
    );
  }
  return [
    {
      id: configModel,
      model: configModel,
      displayName: configModel,
      description: "config.toml 中配置的默认模型",
      hidden: false,
      isDefault: true,
      supportedReasoningEfforts: [],
      defaultReasoningEffort: configEffort,
    },
    ...list.map((m) => applyEffort({ ...m, isDefault: false }, false)),
  ];
}

/**
 * 拉取可用模型列表（幂等），供模型菜单与输入区按钮共用。
 * cwd 传活动会话工作区（与 thread/start 一致）：配置里的 model 会被标为默认；
 * force=true 时重读 config/read 与 model/list（菜单打开时刷新）。
 */
export async function loadModels(force = false, cwd = "") {
  if (store.modelsLoaded && !force) return;
  const [catalog, defaults] = await Promise.all([
    fetchModelCatalog(),
    readEffectiveDefaults(cwd),
  ]);
  // 目录与配置都不可用（含目录失败且无配置模型）：保持现有列表，不误清空
  if (catalog === null && !defaults?.model) return;
  store.models = composeModels(catalog ?? [], defaults);
  store.modelsLoaded = true;
}

/** 确保指定会话的插件缓存已加载（会话级缓存：已加载直接返回，失败可重试） */
export async function ensureThreadPlugins(tab: SessionTab) {
  if (tab.plugins.loaded) return;
  tab.plugins = { plugins: [], loaded: false };
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
    tab.plugins = { plugins: list, loaded: true };
  } catch {
    // 插件列表不可用时保持空，不回退 skills/list
    tab.plugins = { plugins: [], loaded: false };
  }
}

/** 拉取技能列表（会话级缓存，幂等），供 $ 菜单与回显悬浮提示使用 */
export async function ensureSkills(tab: SessionTab) {
  if (tab.skills.loaded) return;
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
    tab.skills = { skills: list, loaded: true };
  } catch {
    // 技能列表不可用时保持空
    tab.skills = { skills: [], loaded: false };
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

/** 当前生效模型 id：传入标签时只取该标签的 model（不读活动标签）→ 默认模型 → 列表首个 → 无可选时抛错 */
export function currentModelId(tab?: Pick<SessionTab, "model">): string {
  // 传入 tab 时严格以该标签为准（后台标签发送不串用活动标签模型）；
  // 未传 tab（"当前会话"语义）才回退活动标签
  const m = tab ? tab.model : (activeSessionTab()?.model ?? null);
  if (m) return m;
  const def = store.models.find((x) => x.isDefault);
  if (def) return def.model;
  const first = store.models[0];
  if (first) return first.model;
  throw new Error("当前没有可用模型，请检查模型列表");
}

/** 当前生效的推理强度：标签显式值优先，否则用默认模型的默认强度 */
export function effectiveEffort(
  tab?: Pick<SessionTab, "model" | "effort">,
): string {
  const session = tab ?? activeSessionTab();
  if (session?.effort) return session.effort;
  const m =
    store.models.find((x) => x.model === session?.model) ??
    store.models.find((x) => x.isDefault);
  return m?.defaultReasoningEffort ?? "";
}

/** 新建会话固化用：解析会话实际生效的模型/推理强度（tab 显式值优先，否则默认模型/默认强度）。 */
export function effectiveSessionModelEffort(
  tab: Pick<SessionTab, "model" | "effort">,
): { model: string | null; effort: string | null } {
  let model = tab.model;
  if (!model) {
    try {
      model = currentModelId(tab);
    } catch {
      model = null; // 模型列表未加载：保持默认
    }
  }
  let effort = tab.effort;
  if (!effort) {
    try {
      effort = effectiveEffort(tab) || null;
    } catch {
      effort = null;
    }
  }
  return { model, effort };
}
