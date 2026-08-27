// 把 sidecar 包装层与其依赖（wechat-channel）打包为单文件 ESM，
// 产物 sidecar-dist/wechat-sidecar.mjs 由 Tauri 资源映射随包分发。

import { build } from "esbuild";
import { existsSync } from "node:fs";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(fileURLToPath(new URL("../package.json", import.meta.url)));
const outdir = path.join(root, "sidecar-dist");
const senderRel = path.join("wechat-channel", "dist", "messaging", "sender.js");
const errorsRel = path.join("wechat-channel", "dist", "errors.js");
const clientRel = path.join("wechat-channel", "dist", "client.js");
const qrLoginRel = path.join("wechat-channel", "dist", "auth", "qr-login.js");

/**
 * 绕过 wechat-channel@1.1.0 sendText 发送前的 markdown 剥离：
 * 把 `const plainText = markdownToPlainText(text);` 替换为直通原文，
 * 让微信端收到原始 Markdown 自行渲染。模式缺失时中止构建，
 * 防止库升级后补丁静默失效。
 */
function patchSenderSource(source) {
  const mark = "const plainText = markdownToPlainText(text);";
  const raw = "const plainText = text;";
  if (source.includes(mark)) {
    return source.replace(mark, raw);
  }
  if (source.includes(raw)) {
    // 已补丁（就地补丁先于 esbuild 读取时）或重复构建：原样直通。
    return source;
  }
  throw new Error(
    `[build-sidecar] 未在 wechat-channel sender.js 中找到 markdown 剥离调用，` +
      `请核对库结构后更新补丁（期望包含: ${mark}）`,
  );
}

/**
 * 收紧 wechat-channel@1.1.0 的会话过期判定：去掉 SESSION_EXPIRED_PATTERN 中的
 * `timeout` 分支，避免长轮询首包的网关/网络瞬时超时被误判为「会话已过期」，
 * 让这类错误走普通失败重试而非停止接收。真正的过期仍由 errcode === -14 或
 * 明确 session/token expired 文案捕获。模式缺失时中止构建。
 */
function patchSessionPattern(source) {
  const from = "token.*expired|timeout";
  const to = "token.*expired";
  if (source.includes(from)) {
    return source.replace(from, to);
  }
  if (source.includes(to)) {
    // 已补丁（就地补丁先于 esbuild 读取时）或重复构建：原样直通。
    return source;
  }
  throw new Error(
    `[build-sidecar] 未在 wechat-channel errors.js 中找到会话过期判定，` +
      `请核对库结构后更新补丁（期望包含: ${from}）`,
  );
}

/**
 * 让 wechat-channel 二维码登录可取消：client.login 把 AbortSignal 透传给 loginWithQr。
 * 模式缺失时中止构建，防止库升级后补丁静默失效。
 */
function patchLoginSignalClient(source) {
  const mark = "timeoutMs: options.timeoutMs,\n            log: this.options.log,";
  const to =
    "timeoutMs: options.timeoutMs,\n            signal: options.signal,\n            log: this.options.log,";
  if (source.includes(mark)) {
    return source.replace(mark, to);
  }
  if (source.includes("signal: options.signal,")) {
    return source;
  }
  throw new Error(
    `[build-sidecar] 未在 wechat-channel client.js 中找到 login 透传点，` +
      `请核对库结构后更新补丁（期望包含: ${mark}）`,
  );
}

/**
 * 让 waitForQrLogin 每轮检查 AbortSignal：abort 后立即结束登录并回「登录已取消」；
 * 并让 loginWithQr 把 signal 传给 waitForQrLogin。幂等 + 模式缺失即报错中止。
 */
function patchLoginSignalQr(source) {
  const abortMark = "    while (Date.now() < deadline) {\n        try {";
  const abortTo =
    "    while (Date.now() < deadline) {\n        if (options.signal?.aborted) { activeLogins.delete(options.sessionKey); return { connected: false, message: \"登录已取消\" }; }\n        try {";
  const sigMark = "        timeoutMs: options.timeoutMs ?? 480_000,\n        log: options.log,";
  const sigTo =
    "        timeoutMs: options.timeoutMs ?? 480_000,\n        signal: options.signal,\n        log: options.log,";
  if (source.includes("options.signal?.aborted") && source.includes("signal: options.signal,")) {
    return source; // 已补丁
  }
  let out = source;
  if (out.includes(abortMark)) {
    out = out.replace(abortMark, abortTo);
  } else if (!out.includes("options.signal?.aborted")) {
    throw new Error(
      `[build-sidecar] 未在 wechat-channel qr-login.js 中找到 waitForQrLogin 轮询点，` +
        `请核对库结构后更新补丁（期望包含: ${abortMark}）`,
    );
  }
  if (out.includes(sigMark)) {
    out = out.replace(sigMark, sigTo);
  } else if (!out.includes("signal: options.signal,")) {
    throw new Error(
      `[build-sidecar] 未在 wechat-channel qr-login.js 中找到 loginWithQr 透传点，` +
        `请核对库结构后更新补丁（期望包含: ${sigMark}）`,
    );
  }
  return out;
}

// 打包期补丁：esbuild 解析到 sender.js 时直接返回替换后的源码。
const markdownRawPlugin = {
  name: "wechat-channel-md-raw",
  setup(build) {
    build.onLoad(
      { filter: /wechat-channel[\\/]dist[\\/]messaging[\\/]sender\.js$/ },
      async (args) => ({
        contents: patchSenderSource(await readFile(args.path, "utf8")),
        loader: "js",
      }),
    );
  },
};

// 打包期补丁：esbuild 解析到 errors.js 时返回收紧后的源码。
const sessionPatternPlugin = {
  name: "wechat-channel-session-pattern",
  setup(build) {
    build.onLoad(
      { filter: /wechat-channel[\\/]dist[\\/]errors\.js$/ },
      async (args) => ({
        contents: patchSessionPattern(await readFile(args.path, "utf8")),
        loader: "js",
      }),
    );
  },
};

// 打包期补丁：client.js 透传 AbortSignal 给 loginWithQr。
const loginSignalClientPlugin = {
  name: "wechat-channel-login-signal-client",
  setup(build) {
    build.onLoad(
      { filter: /wechat-channel[\\/]dist[\\/]client\.js$/ },
      async (args) => ({
        contents: patchLoginSignalClient(await readFile(args.path, "utf8")),
        loader: "js",
      }),
    );
  },
};

// 打包期补丁：qr-login.js 每轮检查 AbortSignal。
const loginSignalQrPlugin = {
  name: "wechat-channel-login-signal-qr",
  setup(build) {
    build.onLoad(
      { filter: /wechat-channel[\\/]dist[\\/]auth[\\/]qr-login\.js$/ },
      async (args) => ({
        contents: patchLoginSignalQr(await readFile(args.path, "utf8")),
        loader: "js",
      }),
    );
  },
};

// 就地补丁：未打包直接跑源码版 sidecar 时走 node_modules 原始模块，
// 同样需要修补，保证两条运行路径行为一致（node_modules 为 gitignored 产物）。
async function applyInPlacePatch(rel, patch, label) {
  const p = path.join(root, "sidecar", "node_modules", ...rel.split(path.sep));
  if (existsSync(p)) {
    await writeFile(p, patch(await readFile(p, "utf8")), "utf8");
    console.log(`[build-sidecar] patched ${path.relative(root, p)}`);
  } else {
    console.warn(`[build-sidecar] 未找到 ${path.relative(root, p)}，跳过就地补丁（${label}）`);
  }
}

await rm(outdir, { recursive: true, force: true });
await mkdir(outdir, { recursive: true });

await applyInPlacePatch(senderRel, patchSenderSource, "markdown");
await applyInPlacePatch(errorsRel, patchSessionPattern, "session");
await applyInPlacePatch(clientRel, patchLoginSignalClient, "login-signal-client");
await applyInPlacePatch(qrLoginRel, patchLoginSignalQr, "login-signal-qr");

await build({
  entryPoints: [path.join(root, "sidecar", "wechat-sidecar.mjs")],
  outfile: path.join(outdir, "wechat-sidecar.mjs"),
  plugins: [markdownRawPlugin, sessionPatternPlugin, loginSignalClientPlugin, loginSignalQrPlugin],
  bundle: true,
  format: "esm",
  platform: "node",
  target: "node18",
  legalComments: "inline",
  logLevel: "info",
});

console.log(`[build-sidecar] done -> ${outdir}`);
