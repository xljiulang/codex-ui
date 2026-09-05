// codex-ui E2E 共享基础设施：进程启动/清理、CDP 客户端、结果汇总。
// 供 scripts/verify-*.mjs 与 scripts/audit-themes.mjs 复用，消除各脚本重复实现。
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
export const APP = path.join(
  repoRoot,
  "src-tauri",
  "target",
  "release",
  "codex-ui.exe",
);

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export const log = (msg) =>
  console.log(`[${new Date().toISOString()}] ${msg}`);

/** 创建临时目录（自动生成唯一前缀目录） */
export function mkTmp(prefix) {
  return fs.mkdtempSync(path.join(os.tmpdir(), prefix));
}

/** 记录一条结果并输出 PASS/FAIL；返回 ok 便于链式断言 */
export function record(results, name, ok, detail = "") {
  results.push({ name, ok, detail });
  log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
  return ok;
}

/** 汇总输出结果，返回是否全部通过 */
export function summarize(results) {
  const pass = results.filter((r) => r.ok).length;
  log(`\n===== 结果汇总: ${pass}/${results.length} 通过 =====`);
  for (const r of results) {
    log(`${r.ok ? "PASS" : "FAIL"} ${r.name}`);
  }
  return results.every((r) => r.ok);
}

/** 探测端口是否已被占用（有 CDP 响应即视为占用） */
export async function portInUse(port) {
  try {
    const res = await fetch(`http://127.0.0.1:${port}/json/list`);
    if (res.ok) return true;
  } catch {}
  return false;
}

/**
 * 启动 release 应用并等待 CDP page target 就绪。
 * 端口占用、进程提前退出、超时都会给出明确诊断信息。
 * 注意：必须可见启动（windowsHide: false），隐藏窗口会阻止 WebView2 初始化。
 */
export async function spawnApp({
  cwd,
  port,
  extraEnv = {},
  timeoutMs = 120000,
}) {
  if (!fs.existsSync(APP)) {
    throw new Error(
      `未找到应用: ${APP}\n请先构建：npm run build && cargo build --release --manifest-path src-tauri/Cargo.toml`,
    );
  }
  if (await portInUse(port)) {
    throw new Error(
      `CDP 端口 ${port} 已被占用：可能已有 codex-ui 实例在运行。` +
        `请先关闭其它实例，或通过环境变量 CODEX_E2E_PORT 换用其它端口。`,
    );
  }
  const child = spawn(APP, [], {
    cwd,
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
      // 仅 E2E 启动暴露 __CODEX_UI_TEST__ 钩子；正常启动不设置，避免
      // openLink/openPathInApp 被测试钩子短路（生产环境文件链接点击无反应）
      CODEX_UI_TEST: "1",
      ...extraEnv,
    },
    stdio: "ignore",
    windowsHide: false,
  });
  const start = Date.now();
  let lastLogAt = start;
  let exited = false;
  child.on("exit", () => {
    exited = true;
  });
  log(`应用已启动 (pid=${child.pid})，等待 CDP ${port}…`);
  let lastError = "";
  while (Date.now() - start < timeoutMs) {
    if (exited) {
      throw new Error(
        `应用进程提前退出（exitCode=${child.exitCode ?? "?"}），无法连接 CDP`,
      );
    }
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/list`);
      const targets = await res.json();
      const page = targets.find((t) => t.type === "page");
      if (page?.webSocketDebuggerUrl) {
        log(`CDP 就绪（耗时 ${((Date.now() - start) / 1000).toFixed(1)}s）`);
        return { child, page };
      }
    } catch (e) {
      lastError = String(e?.message ?? e);
    }
    if (Date.now() - lastLogAt >= 15000) {
      log(
        `等待 CDP… 已等待 ${((Date.now() - start) / 1000).toFixed(0)}s` +
          (lastError ? `（最近错误: ${lastError.slice(0, 120)}）` : ""),
      );
      lastLogAt = Date.now();
    }
    await sleep(500);
  }
  killAppTree(child);
  throw new Error(
    `CDP 连接超时（${timeoutMs}ms）：${lastError || "无响应"}。` +
      `若端口被占用或应用启动失败，请查看上方诊断信息。`,
  );
}

/** 结束应用进程树（Windows taskkill /T /F） */
export function killAppTree(child) {
  if (!child) return;
  try {
    child.kill();
  } catch {}
  try {
    execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
    });
  } catch {}
}

/** 删除 ~/.codex/sessions 中含任一 marker 的会话文件（E2E 自清洁） */
export function cleanupSessions(markers) {
  const root = path.join(os.homedir(), ".codex", "sessions");
  if (!fs.existsSync(root)) return 0;
  const files = [];
  const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.name.endsWith(".jsonl")) files.push(p);
    }
  };
  walk(root);
  let removed = 0;
  for (const f of files) {
    try {
      const content = fs.readFileSync(f, "utf8");
      if (markers.some((m) => m && content.includes(m))) {
        fs.unlinkSync(f);
        removed++;
      }
    } catch {}
  }
  if (removed) log(`已删除 ${removed} 个测试会话文件`);
  return removed;
}

/** 创建 WebView2 CDP 客户端（send/evalJs/waitFor/screenshot/close） */
export async function createClient(wsUrl) {
  const client = { seq: 0, pending: new Map(), ws: null };
  await new Promise((resolve, reject) => {
    client.ws = new WebSocket(wsUrl);
    client.ws.onopen = resolve;
    client.ws.onerror = () => reject(new Error("WebSocket 连接失败"));
  });
  client.ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    if (!msg.id) return;
    const p = client.pending.get(msg.id);
    if (!p) return;
    client.pending.delete(msg.id);
    if (msg.error) p.reject(new Error(JSON.stringify(msg.error)));
    else p.resolve(msg.result);
  };
  client.send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = ++client.seq;
      client.pending.set(id, { resolve, reject });
      client.ws.send(JSON.stringify({ id, method, params }));
    });
  client.evalJs = async (expression) => {
    const r = await client.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (r.exceptionDetails) {
      throw new Error(
        "页面脚本异常: " + JSON.stringify(r.exceptionDetails).slice(0, 500),
      );
    }
    return r.result.value;
  };
  client.waitFor = async (desc, expr, timeoutMs = 15000) => {
    const deadline = Date.now() + timeoutMs;
    let last;
    while (Date.now() < deadline) {
      last = await client.evalJs(expr);
      if (last) return last;
      await sleep(300);
    }
    throw new Error(`等待超时: ${desc}，最后结果: ${JSON.stringify(last)}`);
  };
  client.screenshot = async (name, outDir) => {
    try {
      const r = await client.send("Page.captureScreenshot", { format: "png" });
      const p = path.join(outDir, name);
      fs.writeFileSync(p, Buffer.from(r.data, "base64"));
      log(`截图已保存: ${p}`);
      return p;
    } catch (e) {
      log(`截图失败 ${name}: ${e.message}`);
      return null;
    }
  };
  client.close = () => {
    try {
      client.ws.close();
    } catch {}
  };
  await client.send("Runtime.enable");
  await client.send("Page.enable");
  return client;
}

/** 等待聊天区编辑器（ProseMirror）就绪 */
export async function waitForEditor(client, timeoutMs = 60000) {
  await client.waitFor(
    "编辑器加载",
    `!!document.querySelector(".ProseMirror")`,
    timeoutMs,
  );
}

/**
 * 确保存在可用的会话标签与聊天编辑器：
 * 应用可能启动在“零会话标签”空状态（ad7fc6b 起），先等应用壳就绪，
 * 无编辑器时通过 __CODEX_UI_TEST__.newSession() 创建会话（避开原生目录选择器），
 * 再等编辑器出现。client 只需提供 evalJs（含 waitFor 的 e2e client 亦可）。
 */
export async function ensureSession(client, timeoutMs = 30000) {
  const waitEval = async (desc, expr, waitMs) => {
    const deadline = Date.now() + waitMs;
    let last;
    while (Date.now() < deadline) {
      last = await client.evalJs(expr);
      if (last) return last;
      await sleep(300);
    }
    throw new Error(`等待超时: ${desc}，最后结果: ${JSON.stringify(last)}`);
  };
  await waitEval(
    "应用壳就绪",
    `!!(document.querySelector(".app") || document.querySelector(".ProseMirror"))`,
    timeoutMs,
  );
  const hasEditor = await client.evalJs(
    `!!document.querySelector(".ProseMirror")`,
  );
  if (hasEditor) return;
  const created = await client.evalJs(`(async () => {
    const t = window.__CODEX_UI_TEST__;
    if (!t || typeof t.newSession !== "function") return false;
    await t.newSession();
    return true;
  })()`);
  if (!created) {
    throw new Error("__CODEX_UI_TEST__.newSession 不可用，无法创建会话");
  }
  await waitEval("新会话编辑器", `!!document.querySelector(".ProseMirror")`, 15000);
}

/**
 * 点击文件变更行打开内嵌 diff 标签（主窗口标签页，v-show 切换）：
 * 等可见（非 display:none）diff 窗口的目标路径与内容就绪。返回主页面客户端。
 */
export async function openDiffTab(client, targetText, timeoutMs = 30000) {
  await client.evalJs(`(() => {
    const rows = Array.from(document.querySelectorAll(".assistant-card .change-row.clickable"));
    const row = rows.find((r) => r.textContent.includes(${JSON.stringify(
      targetText,
    )}));
    if (row) row.click();
  })()`);
  await client.waitFor(
    "diff 标签内容就绪",
    `(() => {
      const vis = Array.from(document.querySelectorAll(".diff-pane")).find(
        (w) => w.offsetParent !== null,
      );
      if (!vis) return false;
      const path = vis.querySelector(".diff-pane-path")?.textContent ?? "";
      if (!path.includes(${JSON.stringify(targetText)})) return false;
      return (
        !vis.querySelector(".diff-loading") &&
        (vis.querySelectorAll(".diff-row").length > 0 ||
          !!vis.querySelector(".diff-fallback-note"))
      );
    })()`,
    timeoutMs,
  );
  return client;
}

/** 收尾：执行清理并按结果退出（供 main().then 使用） */
export function finish(results, cleanupFn) {
  try {
    cleanupFn?.();
  } finally {
    process.exit(results.every((r) => r.ok) ? 0 : 1);
  }
}
