import { loadModelProviderConfig, saveModelProviderConfig } from "../useCodex/modelProviderConfig";
import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ModelProviderInfo } from "../../lib/types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const mockedInvoke = vi.mocked(invoke);

/** config/read 返回：用户层原始配置（与真实 app-server 返回形状一致） */
function readResponse(
  modelProviders: Record<string, unknown>,
  extra: Record<string, unknown> = {},
) {
  return {
    config: {},
    layers: [
      {
        name: { type: "user", file: "C:/x/.codex/config.toml", profile: null },
        version: "sha256:abc",
        config: {
          model: "gpt-x",
          model_reasoning_effort: "high",
          model_provider: "deepseek",
          preferred_auth_method: "apikey",
          forced_login_method: "api",
          model_catalog_json: "models.json",
          model_providers: modelProviders,
          ...extra,
        },
        disabledReason: null,
      },
    ],
  };
}

function provider(
  key: string,
  over: Partial<ModelProviderInfo> = {},
): ModelProviderInfo {
  return {
    key,
    name: key,
    base_url: "",
    env_key: "",
    experimental_bearer_token: "",
    wire_api: "responses",
    ...over,
  };
}

describe("useCodex/modelProviderConfig", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  it("loadModelProviderConfig 取用户层 model_providers/model_provider 归一化并保留 raw", async () => {
    mockedInvoke.mockResolvedValue(
      readResponse(
        {
          deepseek: {
            name: "DeepSeek",
            base_url: "https://api.deepseek.com/",
            env_key: "DS",
            experimental_bearer_token: "sk-1",
            wire_api: "responses",
            custom: 42,
          },
          other: { name: "Other", base_url: "https://o.example/v1", wire_api: "chat" },
        },
        { personality: "pragmatic", model_verbosity: "low" },
      ),
    );
    const s = await loadModelProviderConfig();
    expect(s.model).toBe("gpt-x");
    expect(s.model_reasoning_effort).toBe("high");
    expect(s.personality).toBe("pragmatic");
    expect(s.model_verbosity).toBe("low");
    expect(s.model_provider).toBe("deepseek");
    expect(s.preferred_auth_method).toBe("apikey");
    expect(s.forced_login_method).toBe("api");
    expect(s.providers).toEqual([
      {
        key: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/",
        env_key: "DS",
        experimental_bearer_token: "sk-1",
        wire_api: "responses",
      },
      {
        key: "other",
        name: "Other",
        base_url: "https://o.example/v1",
        env_key: "",
        experimental_bearer_token: "",
        wire_api: "chat",
      },
    ]);
    expect(s.raw.deepseek).toEqual({
      name: "DeepSeek",
      base_url: "https://api.deepseek.com/",
      env_key: "DS",
      experimental_bearer_token: "sk-1",
      wire_api: "responses",
      custom: 42,
    });
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "config/read",
      params: { includeLayers: true },
    });
  });

  it("无用户层时回退 config.model_providers，缺失时为空", async () => {
    mockedInvoke.mockResolvedValueOnce({
      config: {
        model_provider: "a",
        model_providers: { a: { name: "A", wire_api: "chat" } },
      },
      layers: [],
    });
    const r1 = await loadModelProviderConfig();
    expect(r1.providers.map((p) => p.key)).toEqual(["a"]);
    expect(r1.model_provider).toBe("a");

    mockedInvoke.mockResolvedValueOnce({ config: {}, layers: null });
    const r2 = await loadModelProviderConfig();
    expect(r2.providers).toEqual([]);
    expect(r2.raw).toEqual({});
  });

  it("saveModelProviderConfig 保留未知字段、整表 replace 并写全部标量", async () => {
    mockedInvoke.mockResolvedValueOnce(
      readResponse({
        deepseek: {
          name: "DeepSeek",
          base_url: "https://api.deepseek.com/",
          custom: 42,
        },
        gone: { name: "Gone" },
      }),
    );
    await saveModelProviderConfig({
      model: "gpt-x",
      model_reasoning_effort: "high",
      personality: "pragmatic",
      model_verbosity: "low",
      model_provider: "deepseek",
      preferred_auth_method: "apikey",
      forced_login_method: "api",
      model_catalog_json: "models.json",
      providers: [
        provider("deepseek", {
          name: "DeepSeek New",
          base_url: "https://api.deepseek.com/",
          experimental_bearer_token: "sk-2",
        }),
      ],
    });
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "config/batchWrite",
      params: {
        edits: [
          {
            keyPath: "model_providers",
            value: {
              deepseek: {
                name: "DeepSeek New",
                base_url: "https://api.deepseek.com/",
                experimental_bearer_token: "sk-2",
                wire_api: "responses",
                custom: 42,
              },
            },
            mergeStrategy: "replace",
          },
          { keyPath: "model_provider", value: "deepseek", mergeStrategy: "replace" },
          { keyPath: "model", value: "gpt-x", mergeStrategy: "replace" },
          {
            keyPath: "model_reasoning_effort",
            value: "high",
            mergeStrategy: "replace",
          },
          {
            keyPath: "personality",
            value: "pragmatic",
            mergeStrategy: "replace",
          },
          {
            keyPath: "model_verbosity",
            value: "low",
            mergeStrategy: "replace",
          },
          {
            keyPath: "preferred_auth_method",
            value: "apikey",
            mergeStrategy: "replace",
          },
          {
            keyPath: "forced_login_method",
            value: "api",
            mergeStrategy: "replace",
          },
          {
            keyPath: "model_catalog_json",
            value: "models.json",
            mergeStrategy: "replace",
          },
        ],
        reloadUserConfig: true,
      },
    });
  });

  it("saveModelProviderConfig 空值删键、列表外提供方删除、标量空串写入", async () => {
    mockedInvoke.mockResolvedValueOnce(
      readResponse({
        a: { name: "A", env_key: "OLD", custom: 1 },
        b: { name: "B" },
      }),
    );
    await saveModelProviderConfig({
      model: "",
      model_reasoning_effort: "",
      personality: "",
      model_verbosity: "",
      model_provider: "a",
      preferred_auth_method: "",
      forced_login_method: "",
      model_catalog_json: "",
      providers: [provider("a", { name: "A" })],
    });
    const call = mockedInvoke.mock.calls.find(
      ([cmd, args]) =>
        cmd === "codex_rpc" &&
        (args as { method?: string } | undefined)?.method === "config/batchWrite",
    )!;
    const edits = (
      call[1] as {
        params: { edits: { keyPath: string; value: unknown }[] };
      }
    ).params.edits;
    // 空值删键（env_key）、未知字段保留（custom）、列表外提供方删除（b）
    expect(edits[0].value).toEqual({
      a: { name: "A", wire_api: "responses", custom: 1 },
    });
    // 标量空串写入（codex 视为未设置）
    expect(edits[2]).toEqual({
      keyPath: "model",
      value: "",
      mergeStrategy: "replace",
    });
    // 空串 personality / model_verbosity 写 null（codex 删除该键，回退内置默认）
    expect(edits[4]).toEqual({
      keyPath: "personality",
      value: null,
      mergeStrategy: "replace",
    });
    expect(edits[5]).toEqual({
      keyPath: "model_verbosity",
      value: null,
      mergeStrategy: "replace",
    });
    // 空目录内容 → model_catalog_json 写 null（codex 删除该键）
    expect(edits[8]).toEqual({
      keyPath: "model_catalog_json",
      value: null,
      mergeStrategy: "replace",
    });
  });

  it("saveModelProviderConfig 校验：非法 key / 重复 / 激活项不存在", async () => {
    const base = {
      model: "x",
      model_reasoning_effort: "",
      personality: "",
      model_verbosity: "",
      model_provider: "",
      preferred_auth_method: "",
      forced_login_method: "",
      model_catalog_json: "",
    };
    await expect(
      saveModelProviderConfig({
        ...base,
        providers: [
          provider("a b", { name: "X", base_url: "u" }),
        ],
      }),
    ).rejects.toThrow("只能包含字母、数字、下划线与连字符");
    await expect(
      saveModelProviderConfig({
        ...base,
        providers: [provider("a"), provider("a")],
      }),
    ).rejects.toThrow("重复");
    await expect(
      saveModelProviderConfig({
        ...base,
        model_provider: "nope",
        providers: [provider("a")],
      }),
    ).rejects.toThrow("不存在");
  });
});
