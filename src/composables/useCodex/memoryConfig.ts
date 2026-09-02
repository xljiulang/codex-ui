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
