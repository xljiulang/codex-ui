// 代理感知的 HTTP 工具（零依赖，只用 node: 内置模块）。
//
// 背景：Node 的全局 fetch 默认**不读** HTTP(S)_PROXY 环境变量（`--use-env-proxy`
// 是 Node 24 才有的运行时开关，本仓库支持的 Node 18 没有它），于是配置了代理的
// 机器上直连 GitHub 会非常慢。这里在 node:http(s) 之上补齐「有代理就走 CONNECT
// 隧道、命中 NO_PROXY 就直连」的行为，供 scripts/ 下的联网脚本复用。
//
// 环境变量优先级（与 curl 一致）：https 目标取 HTTPS_PROXY / https_proxy，
// http 目标取 HTTP_PROXY / http_proxy，两者都缺时统一回退 ALL_PROXY / all_proxy；
// NO_PROXY / no_proxy 命中则一律直连。只读环境变量，不看 Windows 注册表。

import { createWriteStream } from "node:fs";
import http from "node:http";
import https from "node:https";
import net from "node:net";
import { Transform } from "node:stream";
import { pipeline } from "node:stream/promises";
import tls from "node:tls";

/** 与代理建立连接（含 CONNECT 握手）的超时；隧道建成后不设整体超时。 */
const CONNECT_TIMEOUT_MS = 20_000;
/** 自动跟随的重定向上限，超过即报错（同时兜住重定向环）。 */
const MAX_REDIRECTS = 5;
/** CONNECT 响应头体积上限，防止畸形代理把内存撑爆。 */
const MAX_CONNECT_HEAD_BYTES = 64 * 1024;

/** 读环境变量：空白按未设置处理（Windows 上大小写变体是同一个键）。 */
function envValue(env, ...names) {
  for (const name of names) {
    const value = env[name];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return null;
}

/** 目标默认端口（NO_PROXY 条目省略端口时用于比对）。 */
function defaultPort(target) {
  return target.port || (target.protocol === "https:" ? "443" : "80");
}

/** 代理 URL 允许省略 scheme（curl 风格），缺省补 `http://`。 */
function normalizeProxyUrl(raw) {
  const proxy = new URL(raw.includes("://") ? raw : `http://${raw}`);
  if (proxy.protocol !== "http:" && proxy.protocol !== "https:") {
    // socks5:// 之类的代理本模块不支持：明确报错，避免静默连错地方。
    throw new Error(
      `不支持的代理协议 ${proxy.protocol}//（只支持 http / https）: ${raw}`,
    );
  }
  return proxy;
}

/**
 * NO_PROXY 命中判定：支持 `*`、`host`、`.host`、`host:port`、`.host:port`，
 * host 大小写不敏感，命中规则是「相等」或「以 `.host` 结尾」（覆盖子域）。
 * 不做 CIDR 解析：`10.0.0.0/8` 这类条目按字面 host 处理，正常不会命中。
 */
function matchesNoProxy(target, raw) {
  const host = target.hostname.toLowerCase();
  const port = defaultPort(target);
  for (const item of raw.split(",")) {
    const entry = item.trim();
    if (!entry) continue;
    if (entry === "*") return true;
    const match = /^(\[[^\]]+\]|[^:]+)(?::(\d+))?$/.exec(entry);
    if (!match) continue;
    const entryHost = match[1]
      .replace(/^\./, "")
      .replace(/\.$/, "")
      .toLowerCase();
    if (!entryHost) continue;
    if (match[2] && match[2] !== port) continue;
    if (host === entryHost || host.endsWith(`.${entryHost}`)) return true;
  }
  return false;
}

/** 解析目标 URL 该走哪个代理：`{ proxy }`，`null` 表示直连。 */
export function resolveProxy(targetUrl, env = process.env) {
  const target = new URL(targetUrl);
  const noProxy = envValue(env, "NO_PROXY", "no_proxy");
  if (noProxy && matchesNoProxy(target, noProxy)) return { proxy: null };
  const keys =
    target.protocol === "https:"
      ? ["HTTPS_PROXY", "https_proxy"]
      : ["HTTP_PROXY", "http_proxy"];
  return { proxy: envValue(env, ...keys, "ALL_PROXY", "all_proxy") };
}

/** 等待 socket 就绪事件（`connect` / `secureConnect`），失败时透出错误。 */
function waitForReady(socket, event) {
  return new Promise((resolve, reject) => {
    const onReady = () => {
      socket.off("error", onError);
      resolve(socket);
    };
    const onError = (error) => {
      socket.off(event, onReady);
      reject(error);
    };
    socket.once(event, onReady);
    socket.once("error", onError);
  });
}

/** 经代理与目标建立 CONNECT 隧道，返回裸 socket（https 目标由调用方再包 TLS）。 */
async function openTunnel(proxyUrl, host, port) {
  const proxy = normalizeProxyUrl(proxyUrl);
  const proxyPort = Number(
    proxy.port || (proxy.protocol === "https:" ? 443 : 80),
  );
  const socket =
    proxy.protocol === "https:"
      ? tls.connect({
          host: proxy.hostname,
          port: proxyPort,
          servername: proxy.hostname,
        })
      : net.connect(proxyPort, proxy.hostname);

  try {
    await waitForReady(
      socket,
      proxy.protocol === "https:" ? "secureConnect" : "connect",
    );
  } catch (error) {
    socket.destroy();
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`代理连接失败: ${proxyUrl} — ${message}`);
  }

  const head = [
    `CONNECT ${host}:${port} HTTP/1.1`,
    `Host: ${host}:${port}`,
    "Proxy-Connection: Keep-Alive",
  ];
  if (proxy.username || proxy.password) {
    const credentials = `${decodeURIComponent(proxy.username)}:${decodeURIComponent(proxy.password)}`;
    head.push(
      `Proxy-Authorization: Basic ${Buffer.from(credentials).toString("base64")}`,
    );
  }

  return await new Promise((resolve, reject) => {
    let settled = false;
    let buffered = Buffer.alloc(0);
    const fail = (error) => {
      if (settled) return;
      settled = true;
      socket.destroy();
      reject(error);
    };
    const onData = (chunk) => {
      buffered = Buffer.concat([buffered, chunk]);
      const end = buffered.indexOf("\r\n\r\n");
      if (end < 0) {
        if (buffered.length > MAX_CONNECT_HEAD_BYTES) {
          fail(
            new Error(
              `CONNECT ${proxyUrl} 失败: 响应头超过 ${MAX_CONNECT_HEAD_BYTES} 字节`,
            ),
          );
        }
        return;
      }
      socket.off("data", onData);
      const statusLine =
        buffered.subarray(0, end).toString("latin1").split("\r\n")[0] ?? "";
      const status = Number(
        /^HTTP\/1\.[01] (\d{3})/.exec(statusLine)?.[1] ?? 0,
      );
      if (status !== 200) {
        fail(
          new Error(`CONNECT ${proxyUrl} 失败: ${statusLine || "无状态行"}`),
        );
        return;
      }
      socket.setTimeout(0);
      const rest = buffered.subarray(end + 4);
      if (rest.length > 0) socket.unshift(rest);
      settled = true;
      resolve(socket);
    };
    socket.on("data", onData);
    socket.on("error", (error) => {
      fail(new Error(`代理连接失败: ${proxyUrl} — ${error.message}`));
    });
    socket.setTimeout(CONNECT_TIMEOUT_MS, () => {
      fail(new Error(`代理连接超时（${CONNECT_TIMEOUT_MS} ms）: ${proxyUrl}`));
    });
    socket.write(`${head.join("\r\n")}\r\n\r\n`);
  });
}

/** 建立到目标的连接：需要时先经代理隧道，https 目标再包一层 TLS。 */
async function openSocket({ proxy, target }) {
  const isHttps = target.protocol === "https:";
  const host = target.hostname;
  const port = Number(defaultPort(target));
  let socket = null;
  try {
    const raw = proxy ? await openTunnel(proxy, host, port) : null;
    if (isHttps) {
      socket = raw
        ? tls.connect({ socket: raw, servername: host })
        : tls.connect({ host, port, servername: host });
      return await waitForReady(socket, "secureConnect");
    }
    if (raw) return raw;
    socket = net.connect(port, host);
    return await waitForReady(socket, "connect");
  } catch (error) {
    socket?.destroy();
    throw error;
  }
}

/**
 * 发一次 GET：按需走代理隧道（http 目标同样用 CONNECT），返回原始响应。
 *
 * 必须把 `createConnection` 挂在**每次请求新建的 Agent 实例**上：Node 只在请求
 * 没有 `agent` 选项时才看请求级的 `createConnection`（`agent: false` 会直接忽略它，
 * 表现为静默直连），而 Agent 实例上的钩子一定会被调用。`keepAlive: false` 让每条
 * 连接用完即关——脚本是一次性进程，每跳独立连接更好查问题。
 */
function sendRequest(target, proxy, headers) {
  const client = target.protocol === "https:" ? https : http;
  const agent = new client.Agent({ keepAlive: false });
  agent.createConnection = (_options, oncreate) => {
    openSocket({ proxy, target }).then(
      (socket) => oncreate(null, socket),
      (error) => oncreate(error),
    );
  };
  return new Promise((resolve, reject) => {
    const request = client.request(
      target,
      {
        method: "GET",
        headers,
        agent,
      },
      resolve,
    );
    request.once("error", reject);
    request.end();
  });
}

/**
 * 发一次 GET（按需经代理）并自动跟随 3xx，返回最终响应与本次实际使用的代理。
 * 跨源重定向会丢掉调用方给的请求头，避免把凭据带给第三方。
 */
async function getResponse(url, headers, env) {
  let current = new URL(url).toString();
  let currentHeaders = { ...headers };
  for (let hop = 0; ; hop += 1) {
    const target = new URL(current);
    if (target.protocol !== "http:" && target.protocol !== "https:") {
      throw new Error(`不支持的协议: ${target.protocol}`);
    }
    const { proxy } = resolveProxy(current, env);
    const response = await sendRequest(target, proxy, currentHeaders);
    const location = response.headers.location;
    const isRedirect =
      response.statusCode >= 300 &&
      response.statusCode < 400 &&
      Boolean(location);
    if (!isRedirect) return { response, finalUrl: current, proxy };
    response.resume();
    if (hop >= MAX_REDIRECTS) {
      throw new Error(`重定向次数过多（>${MAX_REDIRECTS}）: ${url}`);
    }
    const next = new URL(location, current);
    if (next.origin !== target.origin) currentHeaders = {};
    current = next.toString();
  }
}

/**
 * 流式下载 URL 到本地文件（不整块读入内存）。
 *
 * `onRoute` 在响应头到达、开始收流前回调一次，参数 `{ proxy, url, finalUrl }`，
 * 供调用方打印「直连 / 经代理」；`env` 可注入环境变量（测试用）。
 * 非 2xx 直接抛错且不创建目标文件，重试由调用方负责。
 */
export async function downloadToFile(url, dest, options = {}) {
  const { headers = {}, onRoute, env = process.env } = options;
  const { response, finalUrl, proxy } = await getResponse(url, headers, env);
  try {
    await onRoute?.({ proxy, url, finalUrl });
  } catch (error) {
    response.resume();
    throw error;
  }
  const status = response.statusCode ?? 0;
  if (status < 200 || status >= 300) {
    response.resume();
    throw new Error(`HTTP ${status} ${response.statusMessage}`.trim());
  }
  let bytes = 0;
  const counter = new Transform({
    transform(chunk, _encoding, callback) {
      bytes += chunk.length;
      callback(null, chunk);
    },
  });
  await pipeline(response, counter, createWriteStream(dest));
  return { bytes, finalUrl };
}

/** 读取 URL 的文本响应（同样按需经代理）；状态码交给调用方判断。 */
export async function readText(url, options = {}) {
  const { headers = {}, onRoute, env = process.env } = options;
  const { response, finalUrl, proxy } = await getResponse(url, headers, env);
  await onRoute?.({ proxy, url, finalUrl });
  const chunks = [];
  for await (const chunk of response) chunks.push(chunk);
  return {
    status: response.statusCode ?? 0,
    statusText: response.statusMessage ?? "",
    text: Buffer.concat(chunks).toString("utf8"),
    finalUrl,
  };
}
