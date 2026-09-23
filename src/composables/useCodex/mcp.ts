// useCodex 拆分模块：设置页 MCP 管理（经 app-server 接口 config/read + config/batchWrite 读写，
// 不再由 Rust 直接操作 config.toml）。
import { invoke } from "@tauri-apps/api/core";
import type {
  McpAuthStatus,
  McpEnvEntry,
  McpResourceDetail,
  McpResourceTemplateDetail,
  McpServerDetail,
  McpServerInfo,
  McpToolDetail,
} from "../../lib/types";

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

/** [mcp_servers.<name>] 原始表；保留 codex 自管字段（environment_id 等）与未知字段 */
type RawMcpServer = Record<string, unknown>;

/** omit_tools_from 合法暴露面（ToolExposureSurface） */
const MCP_OMIT_TOOLS = new Set(["direct", "deferred", "code_mode"]);

/** mcpServerStatus/list 原始条目（取 codex 协议子集） */
interface RawMcpServerStatus {
  name?: string;
  serverInfo?: {
    title?: string | null;
    version?: string;
    description?: string | null;
    websiteUrl?: string | null;
    icons?: unknown[] | null;
  } | null;
  tools?: Record<string, RawMcpTool | undefined>;
  resources?: RawMcpResource[];
  resourceTemplates?: RawMcpResourceTemplate[];
  authStatus?: string;
}

interface RawMcpTool {
  name?: string;
  title?: string;
  description?: string;
  inputSchema?: unknown;
}

interface RawMcpResource {
  uri?: string;
  name?: string;
  title?: string;
  description?: string;
  mimeType?: string;
}

interface RawMcpResourceTemplate {
  uriTemplate?: string;
  name?: string;
  title?: string;
  description?: string;
  mimeType?: string;
}

interface RawListMcpServerStatusResponse {
  data?: RawMcpServerStatus[] | null;
  nextCursor?: string | null;
}

/** McpAuthStatus 合法取值集合（缺失/非法统一回落 unknown） */
const MCP_AUTH_STATUSES = new Set<McpAuthStatus>([
  "unknown",
  "unsupported",
  "notLoggedIn",
  "bearerToken",
  "oAuth",
]);

/** 读取 MCP 服务器配置：经 config/read 取用户层原始 [mcp_servers.*] 并归一化为 UI 模型。 */
export async function loadMcpServers(): Promise<{
  servers: McpServerInfo[];
  raw: Record<string, RawMcpServer>;
}> {
  const res = await invoke<RawConfigReadResponse>("codex_rpc", {
    method: "config/read",
    params: { includeLayers: true },
  });
  const userLayer = res?.layers?.find((l) => l.name?.type === "user");
  const rawValue =
    userLayer?.config?.mcp_servers ?? res?.config?.mcp_servers ?? {};
  const rawMap: Record<string, RawMcpServer> =
    rawValue && typeof rawValue === "object" && !Array.isArray(rawValue)
      ? (rawValue as Record<string, RawMcpServer>)
      : {};
  const servers: McpServerInfo[] = [];
  for (const [name, value] of Object.entries(rawMap)) {
    if (!value || typeof value !== "object" || Array.isArray(value)) continue;
    const t = value as RawMcpServer;
    servers.push({
      name,
      command: str(t.command),
      cwd: str(t.cwd).trim() || undefined,
      args: Array.isArray(t.args)
        ? t.args.filter((a): a is string => typeof a === "string")
        : [],
      env: kvEntries(t.env),
      url: str(t.url),
      headers: kvEntries(t.http_headers),
      bearer_token_env_var: str(t.bearer_token_env_var),
      omit_tools_from: omitList(t.omit_tools_from),
    });
  }
  return { servers, raw: { ...rawMap } };
}

/** 读取指定 MCP 服务器的能力状态（工具/资源/模板/认证），找不到返回 null。 */
export async function loadMcpServerStatus(
  name: string,
): Promise<McpServerDetail | null> {
  const res = await invoke<RawListMcpServerStatusResponse>("codex_rpc", {
    method: "mcpServerStatus/list",
    params: { detail: "full" },
  });
  const raw = res?.data?.find((s) => s?.name === name) ?? null;
  return raw ? normalizeMcpServerStatus(raw) : null;
}

/** 归一化 mcpServerStatus/list 条目 → UI 模型（纯函数，便于单测）。 */
export function normalizeMcpServerStatus(
  raw: RawMcpServerStatus,
): McpServerDetail {
  const tools: McpToolDetail[] = [];
  for (const t of Object.values(raw.tools ?? {})) {
    if (!t) continue;
    const name = typeof t.name === "string" ? t.name.trim() : "";
    if (!name) continue;
    tools.push({
      name,
      title: typeof t.title === "string" ? t.title : undefined,
      description:
        typeof t.description === "string" ? t.description : undefined,
      inputSchema: t.inputSchema,
    });
  }
  tools.sort((a, b) => a.name.localeCompare(b.name));

  const resources: McpResourceDetail[] = [];
  for (const r of raw.resources ?? []) {
    if (!r || typeof r.uri !== "string" || !r.uri.trim()) continue;
    resources.push({
      uri: r.uri,
      name: typeof r.name === "string" ? r.name : undefined,
      title: typeof r.title === "string" ? r.title : undefined,
      description:
        typeof r.description === "string" ? r.description : undefined,
      mimeType: typeof r.mimeType === "string" ? r.mimeType : undefined,
    });
  }
  resources.sort((a, b) => (a.uri || "").localeCompare(b.uri || ""));

  const resourceTemplates: McpResourceTemplateDetail[] = [];
  for (const rt of raw.resourceTemplates ?? []) {
    if (!rt || typeof rt.uriTemplate !== "string" || !rt.uriTemplate.trim()) {
      continue;
    }
    resourceTemplates.push({
      uriTemplate: rt.uriTemplate,
      name:
        typeof rt.name === "string" && rt.name.trim()
          ? rt.name.trim()
          : rt.uriTemplate,
      title: typeof rt.title === "string" ? rt.title : undefined,
      description:
        typeof rt.description === "string" ? rt.description : undefined,
      mimeType: typeof rt.mimeType === "string" ? rt.mimeType : undefined,
    });
  }
  resourceTemplates.sort((a, b) =>
    (a.uriTemplate || "").localeCompare(b.uriTemplate || ""),
  );

  const authStatus: McpAuthStatus = MCP_AUTH_STATUSES.has(
    raw.authStatus as McpAuthStatus,
  )
    ? (raw.authStatus as McpAuthStatus)
    : "unknown";
  const serverInfo = raw.serverInfo
    ? {
        title: raw.serverInfo.title,
        version:
          typeof raw.serverInfo.version === "string"
            ? raw.serverInfo.version
            : "",
        description: raw.serverInfo.description,
        websiteUrl: raw.serverInfo.websiteUrl,
        icons: raw.serverInfo.icons,
      }
    : undefined;
  return {
    name: typeof raw.name === "string" ? raw.name : "",
    serverInfo,
    tools,
    resources,
    resourceTemplates,
    authStatus,
  };
}

/** 保存 MCP 服务器配置：整表同步（列表外的服务器删除），保留未知字段，经 config/batchWrite 写入用户层。 */
export async function saveMcpServers(
  servers: McpServerInfo[],
  raw: Record<string, unknown>,
): Promise<void> {
  const merged: Record<string, unknown> = {};
  for (const s of servers) {
    const rawServer = raw[s.name];
    const base: RawMcpServer =
      rawServer && typeof rawServer === "object" && !Array.isArray(rawServer)
        ? { ...(rawServer as RawMcpServer) }
        : {};
    const isHttp = s.url.trim() !== "";
    if (isHttp) {
      setOrRemove(base, "url", s.url.trim());
      setOrRemove(base, "bearer_token_env_var", s.bearer_token_env_var.trim());
      setKv(base, "http_headers", s.headers);
      delete base.command;
      delete base.cwd;
      delete base.args;
      delete base.env;
    } else {
      setOrRemove(base, "command", s.command.trim());
      setOrRemove(base, "cwd", s.cwd?.trim() ?? "");
      const args = s.args.map((a) => a.trim()).filter(Boolean);
      if (args.length) base.args = args;
      else delete base.args;
      setKv(base, "env", s.env);
      delete base.url;
      delete base.http_headers;
      delete base.bearer_token_env_var;
    }
    // omit_tools_from 与 transport 无关：非空写回（过滤非法 + 去重保序），为空删除交回 codex 默认
    const omits = (s.omit_tools_from ?? [])
      .map((x) => x.trim())
      .filter((x) => MCP_OMIT_TOOLS.has(x));
    if (omits.length) base.omit_tools_from = Array.from(new Set(omits));
    else delete base.omit_tools_from;
    merged[s.name.trim()] = base;
  }
  await invoke("codex_rpc", {
    method: "config/batchWrite",
    params: {
      edits: [
        {
          keyPath: "mcp_servers",
          value: merged,
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

function kvEntries(v: unknown): McpEnvEntry[] {
  if (!v || typeof v !== "object" || Array.isArray(v)) return [];
  return Object.entries(v as Record<string, unknown>).map(([key, value]) => ({
    key,
    value: typeof value === "string" ? value : "",
  }));
}

/** 键值表（env / http_headers）：空表整体删除，保留其余字段由调用方控制 */
function setKv(obj: RawMcpServer, key: string, entries: McpEnvEntry[]) {
  const kv: Record<string, string> = {};
  for (const e of entries) {
    const k = e.key.trim();
    if (k) kv[k] = e.value;
  }
  if (Object.keys(kv).length) obj[key] = kv;
  else delete obj[key];
}

function setOrRemove(obj: RawMcpServer, key: string, value: string) {
  if (value) obj[key] = value;
  else delete obj[key];
}

/** 归一化 omit_tools_from：支持数组或单字符串，仅保留合法暴露面，去重保序；缺失/非法 → [] */
function omitList(v: unknown): string[] {
  if (Array.isArray(v)) {
    return Array.from(
      new Set(
        v.filter(
          (x): x is string => typeof x === "string" && MCP_OMIT_TOOLS.has(x),
        ),
      ),
    );
  }
  if (typeof v === "string" && MCP_OMIT_TOOLS.has(v)) return [v];
  return [];
}
