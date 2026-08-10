// codex-ui 目标模式 E2E：首次输入即目标，回合完成/被终止后清除目标并退回执行模式
// 用法: node scripts/verify-goal.mjs
import { spawn, execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = path.join(repoRoot, "src-tauri", "target", "release", "codex-ui.exe");
const CDP_PORT = 9223;
const CDP_BASE = `http://127.0.0.1:${CDP_PORT}`;

const MARKER = "GOMARK_" + crypto.randomBytes(4).toString("hex").toUpperCase();
const testDir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-ui-goal-test-"));
const results = [];
let child = null;
let cdp = null;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function log(msg) {
  console.log(`[${new Date().toISOString()}] ${msg}`);
}

function record(name, ok, detail = "") {
  results.push({ name, ok, detail });
  log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
}

function killApp() {
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
  child = null;
}

function cleanupSessions() {
  const root = path.join(os.homedir(), ".codex", "sessions");
  if (!fs.existsSync(root)) return;
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.name.endsWith(".jsonl")) {
        try {
          if (fs.readFileSync(p, "utf8").includes(MARKER)) fs.unlinkSync(p);
        } catch {}
      }
    }
  };
  walk(root);
}

function cleanup() {
  killApp();
  cleanupSessions();
  try {
    fs.rmSync(testDir, { recursive: true, force: true });
  } catch {}
}

process.on("exit", cleanup);
process.on("SIGINT", () => process.exit(130));
process.on("SIGTERM", () => process.exit(143));

async function waitForCdp(timeoutMs = 90000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`${CDP_BASE}/json/list`);
      const targets = await res.json();
      const page = targets.find((t) => t.type === "page");
      if (page?.webSocketDebuggerUrl) return page;
    } catch {}
    await sleep(500);
  }
  throw new Error("CDP 连接超时");
}

async function createClient(wsUrl) {
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
      await sleep(400);
    }
    throw new Error(`等待超时: ${desc}，最后结果: ${JSON.stringify(last)}`);
  };
  await client.send("Runtime.enable");
  await client.send("Page.enable");
  return client;
}

const evalJs = (expression) => cdp.evalJs(expression);
const waitFor = (desc, expr, timeoutMs) => cdp.waitFor(desc, expr, timeoutMs);

async function setInput(text) {
  await evalJs(`(() => {
    const ed = window.__CODEX_UI_EDITOR__;
    if (!ed) throw new Error("编辑器实例未暴露");
    ed.commands.setTextSelection(ed.state.doc.content.size);
    ed.commands.insertContent(${JSON.stringify(text)});
  })()`);
}

async function switchGoalMode() {
  await waitFor(
    "任务模式按钮可用",
    `!document.querySelector(".task-chip").disabled`,
    60000,
  );
  await evalJs(`(() => {
    document.querySelector(".task-chip").click();
  })()`);
  await waitFor(
    "任务模式菜单",
    `!!document.querySelector(".mode-menu-item")`,
    5000,
  );
  await evalJs(`(() => {
    const item = Array.from(document.querySelectorAll(".mode-menu-item")).find(
      (x) => x.textContent.includes("目标模式"),
    );
    item.click();
  })()`);
  await waitFor(
    "目标模式选中",
    `document.querySelector(".task-chip").textContent.includes("目标模式")`,
    5000,
  );
}

async function clickSend() {
  return evalJs(`(() => {
    const b = document.querySelector("button.send-btn");
    if (!b || b.disabled) return false;
    b.click();
    return true;
  })()`);
}

// 进行中停止：只要还有进行中的回合就点停止（覆盖用户回合与目标自动续跑回合），
// 直到退回执行模式且空闲；返回期间出现的异常 toast。
async function stopUntilIdle(timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let errorSeen = "";
  while (Date.now() < deadline) {
    const state = await evalJs(`(() => ({
      mode: document.querySelector(".task-chip")?.textContent ?? "",
      active: !!document.querySelector(".send-btn.stop"),
      toast: document.querySelector(".toast")?.textContent ?? "",
    }))()`);
    if (state.toast.includes("expected active turn")) {
      errorSeen = state.toast;
      break;
    }
    if (!state.active && state.mode.includes("执行模式")) break;
    if (state.active) {
      await evalJs(`document.querySelector(".send-btn.stop").click()`);
      await sleep(800);
    } else {
      await sleep(500);
    }
  }
  return errorSeen;
}

async function main() {
  if (!fs.existsSync(APP)) throw new Error(`未找到应用: ${APP}`);
  child = spawn(APP, [], {
    cwd: testDir,
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
    },
    stdio: "ignore",
    windowsHide: false,
  });
  const page = await waitForCdp();
  cdp = await createClient(page.webSocketDebuggerUrl);
  await waitFor("编辑器加载", `!!document.querySelector(".ProseMirror")`, 60000);

  // 场景 1：切换目标模式不弹对话框，首条消息即目标，回合完成后退回执行
  await switchGoalMode();
  const modalOpened = await evalJs(`!!document.querySelector(".modal")`);
  record("目标模式: 切换后不自动弹目标对话框", modalOpened === false);

  const goalFile = path.join(testDir, "goal-ok.txt");
  await setInput(`创建文件 ${goalFile}，内容写入 ${MARKER}，完成后停止`);
  const sent = await clickSend();
  if (!sent) throw new Error("发送失败");
  await waitFor(
    "回合完成退回执行模式",
    `document.querySelector(".task-chip").textContent.includes("执行模式")`,
    180000,
  );
  const fileOk =
    fs.existsSync(goalFile) &&
    fs.readFileSync(goalFile, "utf8").includes(MARKER);
  record("目标模式: 实质性目标已完成（文件含标记词）", fileOk);
  const lastAgent = await evalJs(`(() => {
    const agents = document.querySelectorAll(".msg-agent");
    return agents.length ? agents[agents.length - 1].innerText : "";
  })()`);
  record(
    "目标模式: 回合完成后退回执行模式",
    lastAgent.length > 0,
    lastAgent.slice(0, 120),
  );

  // 等待可能的目标自动续跑回合结束（任务模式按钮需可用才能再切换）
  await waitFor(
    "回合全部结束（无进行中回合）",
    `!document.querySelector(".send-btn.stop")`,
    120000,
  );

  // 场景 2：进行中停止（用户回合 + 目标自动续跑回合）后清除目标并退回执行
  await switchGoalMode();
  await setInput(`分析 ${testDir} 目录下的所有文件并给出架构总结，回复 ${MARKER}2`);
  const sent2 = await clickSend();
  if (!sent2) throw new Error("发送失败 2");
  const interruptError = await stopUntilIdle(180000);
  const mode = await evalJs(`document.querySelector(".task-chip")?.textContent ?? ""`);
  record(
    "目标模式: 进行中停止后清除目标并退回执行（无 expected active turn 异常）",
    interruptError === "" && mode.includes("执行模式"),
    (interruptError || mode).slice(0, 120),
  );

  const pass = results.filter((r) => r.ok).length;
  log(`\n===== 结果汇总: ${pass}/${results.length} 通过 =====`);
}

main()
  .then(() => {
    cleanup();
    process.exit(results.every((r) => r.ok) ? 0 : 1);
  })
  .catch((e) => {
    log("E2E 失败: " + e.message);
    cleanup();
    process.exit(2);
  });
