import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type UnlistenFn = () => void;

/** 是否运行在 Tauri（桌面）环境（惰性检测，便于测试切换） */
export function isTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

// ---------------- 远程令牌 ----------------

const TOKEN_KEY = "codex_ui_remote_token";
let remoteTokenCache = "";

function ensureRemoteToken(): string {
  if (remoteTokenCache) return remoteTokenCache;
  const stored = sessionStorage.getItem(TOKEN_KEY);
  if (stored) {
    remoteTokenCache = stored;
    return stored;
  }
  const fromUrl = new URLSearchParams(window.location.search).get("token");
  if (fromUrl) {
    remoteTokenCache = fromUrl;
    sessionStorage.setItem(TOKEN_KEY, fromUrl);
    return fromUrl;
  }
  const asked = (window.prompt("请输入远程访问令牌") ?? "").trim();
  if (asked) {
    remoteTokenCache = asked;
    sessionStorage.setItem(TOKEN_KEY, asked);
  }
  return asked;
}

/** 顶层参数键 camelCase → snake_case（远程 RPC 与 Rust 参数名对齐） */
function toSnakeKeys(args: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(args)) {
    out[k.replace(/[A-Z]/g, (c) => "_" + c.toLowerCase())] = v;
  }
  return out;
}

/** 调用后端命令：Tauri → invoke；远程 Web → POST /rpc */
export function call<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (isTauri()) {
    return args ? invoke<T>(cmd, args) : invoke<T>(cmd);
  }
  return remoteCall<T>(cmd, args);
}

async function remoteCall<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const res = await fetch("/rpc", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${ensureRemoteToken()}`,
    },
    body: JSON.stringify({ cmd, args: args ? toSnakeKeys(args) : {} }),
  });
  if (!res.ok) {
    throw new Error(`远程请求失败（HTTP ${res.status}）`);
  }
  const body = (await res.json()) as { ok: boolean; data?: T; error?: string };
  if (!body.ok) {
    throw new Error(body.error ?? "远程请求失败");
  }
  return body.data as T;
}

// ---------------- 远程事件（WebSocket 单例 + 自动重连） ----------------

type WsHandler = (payload: unknown) => void;
const wsHandlers = new Map<string, Set<WsHandler>>();
let ws: WebSocket | null = null;
let wsReconnectTimer: number | null = null;

function wsConnect() {
  if (
    ws &&
    (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)
  ) {
    return;
  }
  const token = ensureRemoteToken();
  const proto = window.location.protocol === "https:" ? "wss" : "ws";
  const socket = new WebSocket(
    `${proto}://${window.location.host}/events?token=${encodeURIComponent(token)}`,
  );
  ws = socket;
  socket.onmessage = (e) => {
    try {
      const msg = JSON.parse(String(e.data)) as {
        event: string;
        payload: unknown;
      };
      wsHandlers.get(msg.event)?.forEach((h) => h(msg.payload));
    } catch {
      // 忽略无法解析的消息
    }
  };
  socket.onclose = () => {
    if (ws === socket) ws = null;
    if (wsReconnectTimer === null) {
      wsReconnectTimer = window.setTimeout(() => {
        wsReconnectTimer = null;
        wsConnect();
      }, 2000);
    }
  };
}

/** 订阅后端事件：Tauri → listen；远程 Web → WebSocket /events（自动重连） */
export function subscribe<T>(
  event: string,
  handler: (event: { payload: T }) => void,
): Promise<UnlistenFn> {
  if (isTauri()) {
    return listen<T>(event, handler);
  }
  let set = wsHandlers.get(event);
  if (!set) {
    set = new Set();
    wsHandlers.set(event, set);
  }
  const fn: WsHandler = (p) => handler({ payload: p as T });
  set.add(fn);
  wsConnect();
  return Promise.resolve(() => {
    set?.delete(fn);
  });
}

/** 资源 URL：Tauri → asset 协议；远程 Web → /asset（带令牌） */
export function assetUrl(path: string): string {
  if (isTauri()) {
    return convertFileSrc(path);
  }
  return `/asset?path=${encodeURIComponent(path)}&token=${encodeURIComponent(ensureRemoteToken())}`;
}
