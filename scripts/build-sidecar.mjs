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

await rm(outdir, { recursive: true, force: true });
await mkdir(outdir, { recursive: true });

// 就地补丁：未打包直接跑源码版 sidecar（sidecar/wechat-sidecar.mjs）时，
// 走 node_modules 原始模块，同样需要绕过剥离，保证两条运行路径行为一致。
const senderPath = path.join(root, "sidecar", "node_modules", ...senderRel.split(path.sep));
if (existsSync(senderPath)) {
  const patched = patchSenderSource(await readFile(senderPath, "utf8"));
  await writeFile(senderPath, patched, "utf8");
  console.log(`[build-sidecar] patched ${path.relative(root, senderPath)}`);
} else {
  console.warn(`[build-sidecar] 未找到 ${path.relative(root, senderPath)}，跳过就地补丁`);
}

await build({
  entryPoints: [path.join(root, "sidecar", "wechat-sidecar.mjs")],
  outfile: path.join(outdir, "wechat-sidecar.mjs"),
  plugins: [markdownRawPlugin],
  bundle: true,
  format: "esm",
  platform: "node",
  target: "node18",
  legalComments: "inline",
  logLevel: "info",
});

console.log(`[build-sidecar] done -> ${outdir}`);
