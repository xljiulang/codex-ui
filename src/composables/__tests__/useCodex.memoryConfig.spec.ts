import {
  loadMemoryConfig,
  loadMemoryStatus,
  MEMORY_V2_MIN_THREADS,
  saveMemoryConfig,
} from "../useCodex/memoryConfig";
import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockedInvoke = vi.mocked(invoke);

/** config/read 返回：用户层原始配置（与真实 app-server 返回形状一致） */
function readResponse(config: Record<string, unknown>) {
  return {
    config: {},
    layers: [
      {
        name: { type: "user", file: "C:/x/.codex/config.toml", profile: null },
        config,
      },
    ],
  };
}

describe("useCodex/memoryConfig", () => {
  beforeEach(() => mockedInvoke.mockReset());

  it("loadMemoryConfig 缺省读取：enable=false、allowToolGenerate=false", async () => {
    mockedInvoke.mockResolvedValue(readResponse({}));
    expect(await loadMemoryConfig()).toEqual({
      enable: false,
      allowToolGenerate: false,
    });
  });

  it("loadMemoryConfig 读取 features.memories（enable）与 disable_on_external_context（allow）", async () => {
    mockedInvoke.mockResolvedValue(
      readResponse({
        features: { memories: true },
        memories: { disable_on_external_context: false },
      }),
    );
    expect(await loadMemoryConfig()).toEqual({
      enable: true,
      allowToolGenerate: true,
    });
  });

  it("loadMemoryConfig 当 disable_on_external_context 为 true 时 allowToolGenerate 为 false", async () => {
    mockedInvoke.mockResolvedValue(
      readResponse({
        features: { memories: true },
        memories: { disable_on_external_context: true },
      }),
    );
    expect(await loadMemoryConfig()).toEqual({
      enable: true,
      allowToolGenerate: false,
    });
  });

  it("saveMemoryConfig 主开关控三个值、子开关控 disable_on_external_context，并保留其它键", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        return readResponse({
          features: { chronicle: true },
          memories: { foo: "bar" },
        });
      }
      return {};
    });
    await saveMemoryConfig({ enable: true, allowToolGenerate: true });
    const batch = mockedInvoke.mock.calls.find(
      ([, args]) =>
        (args as { method?: string } | undefined)?.method ===
        "config/batchWrite",
    );
    expect(batch).toBeTruthy();
    const params = (batch![1] as { params: unknown }).params as {
      reloadUserConfig: boolean;
      edits: { keyPath: string; value: Record<string, unknown> }[];
    };
    expect(params.reloadUserConfig).toBe(true);
    const features = params.edits.find((e) => e.keyPath === "features");
    expect(features?.value).toEqual({ chronicle: true, memories: true });
    const memories = params.edits.find((e) => e.keyPath === "memories");
    expect(memories?.value).toEqual({
      foo: "bar",
      use_memories: true,
      generate_memories: true,
      disable_on_external_context: false,
    });
  });

  it("saveMemoryConfig 关闭主开关：三主值 false，disable_on_external_context=!allow", async () => {
    mockedInvoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "codex_rpc" && args?.method === "config/read") {
        return readResponse({
          features: { memories: true },
          memories: {
            use_memories: true,
            generate_memories: true,
            disable_on_external_context: false,
          },
        });
      }
      return {};
    });
    await saveMemoryConfig({ enable: false, allowToolGenerate: false });
    const batch = mockedInvoke.mock.calls.find(
      ([, args]) =>
        (args as { method?: string } | undefined)?.method ===
        "config/batchWrite",
    );
    const params = (batch![1] as { params: unknown }).params as {
      edits: { keyPath: string; value: Record<string, unknown> }[];
    };
    const features = params.edits.find((e) => e.keyPath === "features");
    expect(features?.value.memories).toBe(false);
    const memories = params.edits.find((e) => e.keyPath === "memories");
    expect(memories?.value.use_memories).toBe(false);
    expect(memories?.value.generate_memories).toBe(false);
    expect(memories?.value.disable_on_external_context).toBe(true);
  });

  // memory/status 是 codex-cli 0.156.0 新增的实验方法
  it("loadMemoryStatus 归一化 v2 计数与就绪标记，并带上门槛参数", async () => {
    mockedInvoke.mockResolvedValue({
      v2ConsolidatedThreads: 7,
      v2Ready: false,
    });
    expect(await loadMemoryStatus()).toEqual({
      consolidatedThreads: 7,
      ready: false,
    });
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "memory/status",
      params: { minConsolidatedThreads: MEMORY_V2_MIN_THREADS },
    });
  });

  it("loadMemoryStatus 缺字段时回落零值，v2Ready 非 true 即未就绪", async () => {
    mockedInvoke.mockResolvedValue({});
    expect(await loadMemoryStatus()).toEqual({
      consolidatedThreads: 0,
      ready: false,
    });
  });

  it("loadMemoryStatus 老版本 codex（unknown variant）返回 null 供整行隐藏", async () => {
    mockedInvoke.mockRejectedValueOnce(
      new Error(
        "Invalid request: unknown variant `memory/status`, expected one of `initialize`",
      ),
    );
    expect(await loadMemoryStatus()).toBeNull();
  });

  it("loadMemoryStatus 响应非对象时返回 null", async () => {
    mockedInvoke.mockResolvedValue(null);
    expect(await loadMemoryStatus()).toBeNull();
  });
});
