import { loadMcpServers, saveMcpServers } from "../useCodex/mcp";
import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const mockedInvoke = vi.mocked(invoke);

describe("useCodex/mcp", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  it("loadMcpServers 取用户层 mcp_servers 归一化并保留原始表", async () => {
    mockedInvoke.mockResolvedValue({
      config: {},
      layers: [
        {
          name: { type: "user", file: "C:/x/.codex/config.toml", profile: null },
          version: "sha256:abc",
          config: {
            mcp_servers: {
              fs: {
                command: "npx",
                args: ["-y", "server-fs"],
                env: { API_KEY: "sk-1" },
                environment_id: "local",
                enabled: true,
              },
              remote: {
                url: "https://example.com/mcp",
                http_headers: { Authorization: "Bearer x" },
                bearer_token_env_var: "MY_TOKEN",
                environment_id: "local",
              },
            },
          },
          disabledReason: null,
        },
      ],
    });
    const { servers, raw } = await loadMcpServers();
    expect(servers).toEqual([
      {
        name: "fs",
        command: "npx",
        args: ["-y", "server-fs"],
        env: [{ key: "API_KEY", value: "sk-1" }],
        url: "",
        headers: [],
        bearer_token_env_var: "",
        omit_tools_from: [],
      },
      {
        name: "remote",
        command: "",
        args: [],
        env: [],
        url: "https://example.com/mcp",
        headers: [{ key: "Authorization", value: "Bearer x" }],
        bearer_token_env_var: "MY_TOKEN",
        omit_tools_from: [],
      },
    ]);
    expect(raw.fs).toEqual({
      command: "npx",
      args: ["-y", "server-fs"],
      env: { API_KEY: "sk-1" },
      environment_id: "local",
      enabled: true,
    });
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "config/read",
      params: { includeLayers: true },
    });
  });

  it("loadMcpServers 无用户层时回退 config.mcp_servers，缺失时为空", async () => {
    mockedInvoke.mockResolvedValueOnce({
      config: { mcp_servers: { a: { command: "cmd-a" } } },
      layers: [],
    });
    const r1 = await loadMcpServers();
    expect(r1.servers.map((s) => s.name)).toEqual(["a"]);
    expect(r1.raw.a).toEqual({ command: "cmd-a" });

    mockedInvoke.mockResolvedValueOnce({ config: {}, layers: null });
    const r2 = await loadMcpServers();
    expect(r2.servers).toEqual([]);
    expect(r2.raw).toEqual({});
  });

  it("saveMcpServers 保留未知字段并整表 replace 写入 config/batchWrite", async () => {
    await saveMcpServers(
      [
        {
          name: "fs",
          command: "npx",
          args: ["-y", "server-fs"],
          env: [{ key: "API_KEY", value: "sk-2" }],
          url: "",
          headers: [],
          bearer_token_env_var: "",
        },
      ],
      {
        fs: {
          command: "npx",
          environment_id: "local",
          enabled: true,
        },
      },
    );
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "config/batchWrite",
      params: {
        edits: [
          {
            keyPath: "mcp_servers",
            value: {
              fs: {
                command: "npx",
                args: ["-y", "server-fs"],
                env: { API_KEY: "sk-2" },
                environment_id: "local",
                enabled: true,
              },
            },
            mergeStrategy: "replace",
          },
        ],
        reloadUserConfig: true,
      },
    });
  });

  it("saveMcpServers HTTP/STDIO 切换时清理对侧字段", async () => {
    await saveMcpServers(
      [
        {
          name: "srv",
          command: "",
          args: [],
          env: [],
          url: "https://example.com/mcp",
          headers: [{ key: "Authorization", value: "Bearer y" }],
          bearer_token_env_var: "T",
        },
      ],
      { srv: { command: "npx", args: ["a"], env: { X: "1" }, custom: 1 } },
    );
    const value = (
      mockedInvoke.mock.calls[0][1] as {
        params: { edits: { value: Record<string, Record<string, unknown>> }[] };
      }
    ).params.edits[0].value.srv;
    expect(value).toEqual({
      url: "https://example.com/mcp",
      bearer_token_env_var: "T",
      http_headers: { Authorization: "Bearer y" },
      custom: 1,
    });
    expect(value.command).toBeUndefined();
    expect(value.args).toBeUndefined();
    expect(value.env).toBeUndefined();
  });

  it("saveMcpServers 重建 env 只保留 UI 条目并删除列表外服务器", async () => {
    await saveMcpServers(
      [
        {
          name: "srv",
          command: "npx",
          args: [],
          env: [{ key: "K", value: "v" }],
          url: "",
          headers: [],
          bearer_token_env_var: "",
        },
      ],
      { srv: { command: "old", env: { K: "old", EXTRA: "x" } }, gone: { command: "x" } },
    );
    const value = (
      mockedInvoke.mock.calls[0][1] as {
        params: { edits: { value: Record<string, Record<string, unknown>> }[] };
      }
    ).params.edits[0].value;
    expect(value.srv).toEqual({ command: "npx", env: { K: "v" } });
    expect(value.gone).toBeUndefined();
  });

  it("saveMcpServers 空列表写空对象", async () => {
    await saveMcpServers([], { old: { command: "x" } });
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "config/batchWrite",
      params: {
        edits: [
          { keyPath: "mcp_servers", value: {}, mergeStrategy: "replace" },
        ],
        reloadUserConfig: true,
      },
    });
  });

  it("loadMcpServers 归一化 omit_tools_from：数组去重过滤、单字符串、缺失为空", async () => {
    mockedInvoke.mockResolvedValueOnce({
      config: {},
      layers: [
        {
          name: { type: "user", file: "C:/x/.codex/config.toml", profile: null },
          version: "sha256:abc",
          config: {
            mcp_servers: {
              a: {
                command: "ca",
                omit_tools_from: ["deferred", "direct", "deferred", "bogus"],
              },
              b: { command: "cb", omit_tools_from: "code_mode" },
              c: { command: "cc", omit_tools_from: ["not-a-surface"] },
              d: { command: "cd" },
            },
          },
          disabledReason: null,
        },
      ],
    });
    const { servers } = await loadMcpServers();
    expect(servers.find((s) => s.name === "a")?.omit_tools_from).toEqual([
      "deferred",
      "direct",
    ]);
    expect(servers.find((s) => s.name === "b")?.omit_tools_from).toEqual([
      "code_mode",
    ]);
    expect(servers.find((s) => s.name === "c")?.omit_tools_from).toEqual([]);
    expect(servers.find((s) => s.name === "d")?.omit_tools_from).toEqual([]);
  });

  it("saveMcpServers 非空写 omit_tools_from，空删除并保留其余未知字段", async () => {
    await saveMcpServers(
      [
        {
          name: "on",
          command: "npx",
          args: [],
          env: [],
          url: "",
          headers: [],
          bearer_token_env_var: "",
          omit_tools_from: ["deferred", "deferred", "bogus"],
        },
        {
          name: "off",
          command: "npx",
          args: [],
          env: [],
          url: "",
          headers: [],
          bearer_token_env_var: "",
          omit_tools_from: [],
        },
      ],
      {
        on: { command: "old", custom: 1 },
        off: { command: "old", omit_tools_from: ["deferred"], custom: 2 },
      },
    );
    const value = (
      mockedInvoke.mock.calls[0][1] as {
        params: { edits: { value: Record<string, Record<string, unknown>> }[] };
      }
    ).params.edits[0].value;
    expect(value.on).toEqual({ command: "npx", omit_tools_from: ["deferred"], custom: 1 });
    expect(value.off).toEqual({ command: "npx", custom: 2 });
    expect(value.off.omit_tools_from).toBeUndefined();
  });

  it("loadMcpServers 归一化 cwd：trim 后有值保留，缺失/空为 undefined", async () => {
    mockedInvoke.mockResolvedValue({
      config: {},
      layers: [
        {
          name: { type: "user", file: "C:/x/.codex/config.toml", profile: null },
          version: "sha256:abc",
          config: {
            mcp_servers: {
              withCwd: { command: "cmd-a", cwd: "  C:/work  " },
              emptyCwd: { command: "cmd-b", cwd: "   " },
              noCwd: { command: "cmd-c" },
            },
          },
          disabledReason: null,
        },
      ],
    });
    const { servers } = await loadMcpServers();
    expect(servers.find((s) => s.name === "withCwd")?.cwd).toBe("C:/work");
    expect(servers.find((s) => s.name === "emptyCwd")?.cwd).toBeUndefined();
    expect(servers.find((s) => s.name === "noCwd")?.cwd).toBeUndefined();
  });

  it("saveMcpServers stdio 写 cwd、空值删除；HTTP 删除 cwd 并保留未知字段", async () => {
    await saveMcpServers(
      [
        {
          name: "stdio-a",
          command: "npx",
          cwd: "  D:/work  ",
          args: ["-y", "srv"],
          env: [],
          url: "",
          headers: [],
          bearer_token_env_var: "",
        },
        {
          name: "stdio-b",
          command: "npx",
          cwd: "   ",
          args: [],
          env: [],
          url: "",
          headers: [],
          bearer_token_env_var: "",
        },
        {
          name: "http-x",
          command: "",
          cwd: "D:/ignored",
          args: [],
          env: [],
          url: "https://example.com/mcp",
          headers: [],
          bearer_token_env_var: "",
        },
      ],
      {
        "stdio-b": { command: "old", cwd: "D:/old", custom: 1 },
        "http-x": { command: "old", cwd: "D:/old", custom: 2 },
      },
    );
    const value = (
      mockedInvoke.mock.calls[0][1] as {
        params: { edits: { value: Record<string, Record<string, unknown>> }[] };
      }
    ).params.edits[0].value;
    expect(value["stdio-a"].cwd).toBe("D:/work");
    expect(value["stdio-b"]).toEqual({ command: "npx", custom: 1 });
    expect(value["stdio-b"].cwd).toBeUndefined();
    expect(value["http-x"]).toEqual({
      url: "https://example.com/mcp",
      custom: 2,
    });
    expect(value["http-x"].cwd).toBeUndefined();
  });
});
