// useCodex 拆分模块：本地记忆（经 app-server 接口 config/read + config/batchWrite 读写
// codex 配置 [features].memories 与 [memories].use_memories / generate_memories；镜像 useCodex/mcp.ts）。
import { invoke } from "@tauri-apps/api/core";

/** config/read 返回的配置层（取子集，与服务端 schema 对齐） */
interface RawConfigLayer {
  name?: { type?: string; file?: string | null; profile?: string | null };
  config?: Record<string, unknown>;
}

interface RawConfigReadResponse {
  config?: Record<string, unknown>;
  layers?: RawConfigLayer[] | null;
}

/** 本地记忆设置状态：enable=启用本地记忆；allowToolGenerate=允许工具辅助生成记忆 */
export interface MemoryConfigState {
  enable: boolean;
  allowToolGenerate: boolean;
}

/** 读取用户层原始配置（保存时作为未知字段基底） */
async function readUserLayerConfig(): Promise<Record<string, unknown>> {
  const res = await invoke<RawConfigReadResponse>("codex_rpc", {
    method: "config/read",
    params: { includeLayers: true },
  });
  const userLayer = res?.layers?.find((l) => l?.name?.type === "user");
  return userLayer?.config ?? res?.config ?? {};
}

/** 读取本地记忆配置：经 config/read 取用户层 [features].memories 与 [memories].* 并归一化。 */
export async function loadMemoryConfig(): Promise<MemoryConfigState> {
  const cfg = await readUserLayerConfig();
  const features = asObj(cfg.features);
  const memories = asObj(cfg.memories);
  // 主开关「启用本地记忆」= [features].memories 全局门；
  // 子开关「允许工具辅助生成记忆」= 未禁用外部上下文（disable_on_external_context 非 true）。
  return {
    enable: features.memories === true,
    allowToolGenerate: memories.disable_on_external_context === false,
  };
}

/** 保存本地记忆配置：合并保留 features/memories 其它键后经 config/batchWrite 写回并热重载 */
export async function saveMemoryConfig(edit: MemoryConfigState): Promise<void> {
  const cfg = await readUserLayerConfig();
  const features = asObj(cfg.features);
  const memories = asObj(cfg.memories);
  // 主开关同时控制三个值：features.memories / use_memories / generate_memories；
  // 子开关控制 disable_on_external_context（允许=false、不允许=true）。
  features.memories = edit.enable;
  memories.use_memories = edit.enable;
  memories.generate_memories = edit.enable;
  memories.disable_on_external_context = !edit.allowToolGenerate;

  await invoke("codex_rpc", {
    method: "config/batchWrite",
    params: {
      edits: [
        { keyPath: "features", value: features, mergeStrategy: "replace" },
        { keyPath: "memories", value: memories, mergeStrategy: "replace" },
      ],
      reloadUserConfig: true,
    },
  });
}

function asObj(v: unknown): Record<string, unknown> {
  return v && typeof v === "object" && !Array.isArray(v)
    ? { ...(v as Record<string, unknown>) }
    : {};
}

/** 记忆 v2 就绪状态（app-server 0.156.0+ 的 `memory/status`） */
export interface MemoryV2Status {
  /** 已整合的会话数（达到 `MEMORY_V2_MIN_THREADS` 才就绪） */
  consolidatedThreads: number;
  /** v2 是否已就绪 */
  ready: boolean;
}

/** `memory/status` 的就绪门槛（服务端默认值；小于该值 v2 未启用）。 */
export const MEMORY_V2_MIN_THREADS = 20;

/**
 * 读取记忆 v2 状态（只读展示用）。
 *
 * `memory/status` 是 codex-cli 0.156.0 新增的实验方法：老版本会回
 * `unknown variant` 之类的错误，此时返回 null，由调用方整行隐藏。
 */
export async function loadMemoryStatus(): Promise<MemoryV2Status | null> {
  try {
    const res = await invoke<{
      v2ConsolidatedThreads?: unknown;
      v2Ready?: unknown;
    }>("codex_rpc", {
      method: "memory/status",
      params: { minConsolidatedThreads: MEMORY_V2_MIN_THREADS },
    });
    if (!res || typeof res !== "object") return null;
    return {
      consolidatedThreads:
        typeof res.v2ConsolidatedThreads === "number"
          ? res.v2ConsolidatedThreads
          : 0,
      ready: res.v2Ready === true,
    };
  } catch {
    // 老版本 codex 没有该方法：静默隐藏状态行
    return null;
  }
}
