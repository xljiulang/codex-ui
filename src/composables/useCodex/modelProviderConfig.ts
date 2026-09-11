// useCodex 拆分模块：模型提供方配置（经 app-server 接口 config/read + config/batchWrite 读写，
// 不再由 Rust 直接操作 config.toml；镜像 useCodex/mcp.ts）。
import { invoke } from "@tauri-apps/api/core";
import type { ModelConfigUiEdit, ModelProviderInfo } from "../../lib/types";

/** config/read 返回的配置层（取子集，与服务端 schema 对齐） */
interface RawConfigLayer {
  name?: { type?: string; file?: string | null; profile?: string | null };
  version?: string;
  config?: Record<string, unknown>;
  disabledReason?: string | null;
}

interface RawConfigReadResponse {
  config?: Record<string, unknown>;
  layers?: RawConfigLayer[] | null;
}

/** 模型提供方卡片读取结果：顶层标量 + 全部 [model_providers.*] + 原始表基底。 */
export interface ModelProviderConfigState {
  model: string;
  model_reasoning_effort: string;
  /** 推理摘要 model_reasoning_summary（auto/concise/detailed/none），空串表示未配置 */
  model_reasoning_summary: string;
  /** 回复风格 personality（friendly/pragmatic/none），空串表示未配置 */
  personality: string;
  /** 输出详细程度 model_verbosity（low/medium/high），空串表示未配置 */
  model_verbosity: string;
  model_provider: string;
  preferred_auth_method: string;
  forced_login_method: string;
  providers: ModelProviderInfo[];
  /** 用户层原始 [model_providers.*] 表（保存时作为未知字段基底） */
  raw: Record<string, unknown>;
}

/** 读取模型提供方配置：经 config/read 取用户层原始配置并归一化。 */
export async function loadModelProviderConfig(): Promise<ModelProviderConfigState> {
  const res = await invoke<RawConfigReadResponse>("codex_rpc", {
    method: "config/read",
    params: { includeLayers: true },
  });
  const userLayer = res?.layers?.find((l) => l?.name?.type === "user");
  const cfg = userLayer?.config ?? res?.config ?? {};
  const rawValue = cfg.model_providers;
  const rawMap: Record<string, unknown> =
    rawValue && typeof rawValue === "object" && !Array.isArray(rawValue)
      ? (rawValue as Record<string, unknown>)
      : {};
  const providers: ModelProviderInfo[] = [];
  for (const [key, value] of Object.entries(rawMap)) {
    if (!value || typeof value !== "object" || Array.isArray(value)) continue;
    const t = value as Record<string, unknown>;
    providers.push({
      key,
      name: str(t.name),
      base_url: str(t.base_url),
      env_key: str(t.env_key),
      experimental_bearer_token: str(t.experimental_bearer_token),
      wire_api: str(t.wire_api),
      requires_openai_auth: !!t.requires_openai_auth,
    });
  }
  return {
    model: str(cfg.model),
    model_reasoning_effort: str(cfg.model_reasoning_effort),
    model_reasoning_summary: str(cfg.model_reasoning_summary),
    personality: str(cfg.personality),
    model_verbosity: str(cfg.model_verbosity),
    model_provider: str(cfg.model_provider),
    preferred_auth_method: str(cfg.preferred_auth_method),
    forced_login_method: str(cfg.forced_login_method),
    providers,
    raw: { ...rawMap },
  };
}

/** 保存模型提供方配置：整表同步（列表外的提供方删除），保留未知字段，
 *  顶层标量一并写入，经 config/batchWrite 热重载。 */
export async function saveModelProviderConfig(edit: ModelConfigUiEdit): Promise<void> {
  // 校验（与旧 Rust 直写语义一致）：标识合法且不重复，激活项必须存在。
  const seen: string[] = [];
  for (const p of edit.providers) {
    const key = p.key.trim();
    if (!key) throw new Error("提供方标识不能为空");
    if (!/^[A-Za-z0-9_-]+$/.test(key)) {
      throw new Error(`提供方标识「${key}」只能包含字母、数字、下划线与连字符`);
    }
    // codex 0.149.x 已不支持 chat：写入会让整份配置失效（用户层读不到）
    const wireApi = p.wire_api.trim();
    if (wireApi && wireApi !== "responses") {
      throw new Error(
        `提供方「${key}」的 wire_api 仅支持 "responses"（codex 0.149.x 已不支持 chat，当前为 "${wireApi}"）`,
      );
    }
    if (seen.includes(key)) throw new Error(`提供方标识「${key}」重复`);
    seen.push(key);
  }
  const active = edit.model_provider.trim();
  if (active && !seen.includes(active)) {
    throw new Error(`激活的提供方「${active}」不存在`);
  }

  // 以当前用户层原始表为基底合并已知字段（保留 codex 自管/未知字段）。
  const current = await loadModelProviderConfig();
  const merged: Record<string, unknown> = {};
  for (const p of edit.providers) {
    const key = p.key.trim();
    const rawBase = current.raw[key];
    const base: Record<string, unknown> =
      rawBase && typeof rawBase === "object" && !Array.isArray(rawBase)
        ? { ...(rawBase as Record<string, unknown>) }
        : {};
    setOrRemove(base, "name", p.name.trim());
    setOrRemove(base, "base_url", p.base_url.trim());
    setOrRemove(base, "env_key", p.env_key.trim());
    setOrRemove(base, "experimental_bearer_token", p.experimental_bearer_token.trim());
    setOrRemove(base, "wire_api", p.wire_api.trim());
    setOrRemoveBool(base, "requires_openai_auth", !!p.requires_openai_auth);
    merged[key] = base;
  }

  // model_catalog_json：非空写绝对路径；空/非字符串视为未配置，写 null 让 codex 删除该键
  const catalogRaw = edit.model_catalog_json?.trim();
  await invoke("codex_rpc", {
    method: "config/batchWrite",
    params: {
      edits: [
        { keyPath: "model_providers", value: merged, mergeStrategy: "replace" },
        {
          // 空值删键：写空串会让 codex 判定配置无效（用户层直接从 config/read 消失），
          // 进而让设置页读到空提供方列表、下次保存把 model_providers 整表覆盖为空
          keyPath: "model_provider",
          value: active || null,
          mergeStrategy: "replace",
        },
        // model：空串写 null 让 codex 删除该键（回退默认）
        { keyPath: "model", value: edit.model.trim() || null, mergeStrategy: "replace" },
        {
          keyPath: "model_reasoning_effort",
          // 空值删键：写空串会被 codex 拒绝（reasoning_effort must not be empty），导致保存失败
          value: edit.model_reasoning_effort.trim() || null,
          mergeStrategy: "replace",
        },
        {
          // model_reasoning_summary：空串写 null 让 codex 删除该键（回退模型条目默认值）
          keyPath: "model_reasoning_summary",
          value: edit.model_reasoning_summary.trim() || null,
          mergeStrategy: "replace",
        },
        {
          // personality / model_verbosity：空串写 null 让 codex 删除该键（回退内置默认）
          keyPath: "personality",
          value: edit.personality.trim() || null,
          mergeStrategy: "replace",
        },
        {
          keyPath: "model_verbosity",
          value: edit.model_verbosity.trim() || null,
          mergeStrategy: "replace",
        },
        {
          // preferred_auth_method / forced_login_method：空串写 null 让 codex 删除该键（回退默认）
          keyPath: "preferred_auth_method",
          value: edit.preferred_auth_method.trim() || null,
          mergeStrategy: "replace",
        },
        {
          keyPath: "forced_login_method",
          value: edit.forced_login_method.trim() || null,
          mergeStrategy: "replace",
        },
        {
          keyPath: "model_catalog_json",
          value: catalogRaw || null,
          mergeStrategy: "replace",
        },
      ],
      reloadUserConfig: true,
    },
  });
}

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}

/** 写入字符串值；空串时移除该键（与旧 Rust 直写语义一致）。 */
function setOrRemove(obj: Record<string, unknown>, key: string, value: string) {
  if (value) obj[key] = value;
  else delete obj[key];
}

function setOrRemoveBool(obj: Record<string, unknown>, key: string, value: boolean) {
  if (value) obj[key] = true;
  else delete obj[key];
}
