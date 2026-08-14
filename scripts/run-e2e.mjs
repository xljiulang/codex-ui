// codex-ui E2E 一键编排：构建（可选）→ 串行运行探针 → 汇总。
// 用法:
//   node scripts/run-e2e.mjs                # 直接运行（需已构建 release）
//   node scripts/run-e2e.mjs --build        # 先 npm run build + cargo build --release
//   node scripts/run-e2e.mjs --only goal    # 只跑目标探针（逗号分隔多个）
//   node scripts/run-e2e.mjs --skip audit   # 跳过主题审计
//   node scripts/run-e2e.mjs --continue     # 失败后继续跑剩余探针
//   node scripts/run-e2e.mjs --audit-chat   # 主题审计跑 chat 阶段（默认 static）
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { APP, log } from "./lib/e2e.mjs";

const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
const has = (name) => process.argv.includes(`--${name}`);

const PROBES = [
  { name: "core", script: "verify-core.mjs", port: 9222, args: [] },
  { name: "diff", script: "verify-diff.mjs", port: 9222, args: [] },
  { name: "goal", script: "verify-goal.mjs", port: 9223, args: [] },
  { name: "mentions", script: "verify-mentions.mjs", port: 9222, args: [] },
  {
    name: "audit",
    script: "audit-themes.mjs",
    port: 9225,
    args: [has("audit-chat") ? "--phase=chat" : "--phase=static"],
  },
];

function run(cmd, args, opts = {}) {
  log(`运行: ${cmd} ${args.join(" ")}`);
  const r = spawnSync(cmd, args, {
    stdio: "inherit",
    cwd: repoRoot,
    env: process.env,
    ...opts,
  });
  if (r.error) throw r.error;
  return r.status ?? 1;
}

async function main() {
  if (has("help") || process.argv.includes("-h")) {
    console.log(`codex-ui E2E 编排
用法: node scripts/run-e2e.mjs [--build] [--only <name>] [--skip <name>] [--continue] [--audit-chat]
  --build       先执行 npm run build + cargo build --release
  --only        只运行指定探针（逗号分隔: core,diff,goal,mentions,audit）
  --skip        跳过指定探针
  --continue    单个探针失败后继续运行剩余探针
  --audit-chat  主题审计改用 chat 阶段（默认 static）`);
    return 0;
  }

  if (has("build")) {
    log("步骤 1/2: 构建前端…");
    if (run("npm", ["run", "build"]) !== 0) {
      log("前端构建失败");
      return 1;
    }
    log("步骤 2/2: 构建 Rust release…");
    if (
      run("cargo", [
        "build",
        "--release",
        "--manifest-path",
        "src-tauri/Cargo.toml",
      ]) !== 0
    ) {
      log("Rust release 构建失败");
      return 1;
    }
  }

  const codexCheck = spawnSync("codex", ["--version"], { stdio: "ignore" });
  if (codexCheck.error) {
    log("警告: 未检测到 codex CLI，探针需要真实 codex app-server + 模型调用");
  }

  if (!fs.existsSync(APP)) {
    log(`未找到 release 应用: ${APP}\n请先运行 node scripts/run-e2e.mjs --build`);
    return 1;
  }

  const only = arg("only", "");
  const skip = arg("skip", "");
  const results = [];
  for (const p of PROBES) {
    if (only && !only.split(",").includes(p.name)) continue;
    if (skip && skip.split(",").includes(p.name)) continue;
    log(`\n===== E2E 探针: ${p.name} =====`);
    const env = { ...process.env, CODEX_E2E_PORT: String(p.port) };
    const r = spawnSync(
      process.execPath,
      [path.join("scripts", p.script), ...p.args],
      { stdio: "inherit", cwd: repoRoot, env },
    );
    const ok = r.status === 0;
    results.push({ name: p.name, ok, code: r.status });
    log(`${ok ? "PASS" : "FAIL"} ${p.name} (exit=${r.status ?? "?"})`);
    if (!ok && !has("continue")) {
      log("检测到失败，停止后续探针（--continue 可继续）");
      break;
    }
  }

  const pass = results.filter((r) => r.ok).length;
  log(`\n===== E2E 汇总: ${pass}/${results.length} 通过 =====`);
  for (const r of results) {
    log(`${r.ok ? "PASS" : "FAIL"} ${r.name}`);
  }
  return results.every((r) => r.ok) ? 0 : 1;
}

main()
  .then((code) => process.exit(code))
  .catch((e) => {
    log("E2E 编排失败: " + e.message);
    process.exit(2);
  });
