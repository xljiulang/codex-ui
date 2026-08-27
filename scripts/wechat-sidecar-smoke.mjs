// 微信桥 sidecar 冒烟自检：启动进程 → 下发 init → 收 ready/error 事件 → 退出。
// 用法：node scripts/wechat-sidecar-smoke.mjs [stateDir] [scriptPath]
// 默认跑源码版（sidecar/），可传入 sidecar-dist 路径验证打包产物。

import { spawn } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { mkdtempSync } from "node:fs";
import { fileURLToPath } from "node:url";

const script =
  (process.argv[3] ? path.resolve(process.argv[3]) : undefined) ??
  fileURLToPath(new URL("../sidecar/wechat-sidecar.mjs", import.meta.url));
const cwd = fileURLToPath(new URL("../sidecar/", import.meta.url));
const stateDir = process.argv[2] ?? mkdtempSync(path.join(os.tmpdir(), "codex-ui-wechat-"));

const child = spawn(process.execPath, [script], {
  stdio: ["pipe", "pipe", "inherit"],
  cwd,
});

let buf = "";
child.stdout.on("data", (d) => {
  buf += d.toString("utf8");
  let i;
  while ((i = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, i).trim();
    buf = buf.slice(i + 1);
    if (!line) continue;
    try {
      const evt = JSON.parse(line);
      console.log("[event]", JSON.stringify(evt));
      // 收到首个 init 后的 ready 即停止交互（验证幂等由 Rust 侧单次下发保证）
    } catch {
      console.log("[raw]", line);
    }
  }
});

child.on("exit", (code) => {
  console.log(`[exit] code=${code}`);
  const ok = code === 0 || code === null; // 进程被 kill 时为 null
  if (!ok) process.exit(1);
});

child.stdin.write(JSON.stringify({ cmd: "init", stateDir }) + "\n");

setTimeout(() => {
  child.kill();
  setTimeout(() => process.exit(0), 200);
}, 4000);
