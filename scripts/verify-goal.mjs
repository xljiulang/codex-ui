// codex-ui 线程级目标 E2E：旗子图标 + × 取消，会话前可预填，填写即生效
// 流程：无会话预填/取消预填 → 发送首条消息自动挂载目标 → 服务端围绕目标自动续跑完成（文件标记）
//       → 状态颜色变为已完成 → × 取消目标 → 再次设置后点停止先清目标
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
const MARKER2 = "GOMARK2_" + crypto.randomBytes(4).toString("hex").toUpperCase();
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
          const text = fs.readFileSync(p, "utf8");
          if (text.includes(MARKER) || text.includes(MARKER2)) fs.unlinkSync(p);
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

/** 在富文本输入框写入文本 */
async function setEditorText(text) {
  await evalJs(`(() => {
    const ed = window.__CODEX_UI_EDITOR__;
    if (!ed) throw new Error("编辑器实例未暴露");
    ed.commands.setTextSelection(ed.state.doc.content.size);
    ed.commands.insertContent(${JSON.stringify(text)});
  })()`);
}

/** 在设置目标弹层的 textarea 写入文本（触发 v-model） */
async function setGoalInput(text) {
  await evalJs(`(() => {
    const ta = document.querySelector(".goal-input");
    if (!ta) throw new Error("目标输入框未找到");
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLTextAreaElement.prototype,
      "value",
    ).set;
    setter.call(ta, ${JSON.stringify(text)});
    ta.dispatchEvent(new Event("input", { bubbles: true }));
  })()`);
}

async function clickSend() {
  return evalJs(`(() => {
    const b = document.querySelector("button.send-btn");
    if (!b || b.disabled) return false;
    b.click();
    return true;
  })()`);
}

/** 点击目标旗子（无会话也可点），等待弹层出现 */
async function clickGoalFlag() {
  await waitFor(
    "目标旗子可用",
    `(() => {
      const b = document.querySelector(".goal-icon-btn");
      return !!b && !b.disabled;
    })()`,
    60000,
  );
  await evalJs(`document.querySelector(".goal-icon-btn").click()`);
  await waitFor("设置目标弹层", `!!document.querySelector(".goal-menu")`, 5000);
}

/** 在弹层填入目标并按 Enter 确认（填写即生效） */
async function fillGoalAndConfirm(text) {
  await setGoalInput(text);
  await evalJs(`(() => {
    const ta = document.querySelector(".goal-input");
    ta.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
  })()`);
  await waitFor("弹层关闭", `!document.querySelector(".goal-menu")`, 5000);
  await waitFor(
    "目标已挂载（× 出现）",
    `!!document.querySelector(".goal-clear-btn")`,
    5000,
  );
}

/** × 直接取消目标（不弹确认） */
async function cancelGoalViaX() {
  await waitFor(
    "× 按钮存在",
    `!!document.querySelector(".goal-clear-btn")`,
    5000,
  );
  await evalJs(`document.querySelector(".goal-clear-btn").click()`);
  await waitFor(
    "目标已清除（× 消失）",
    `!document.querySelector(".goal-clear-btn")`,
    10000,
  );
}

/** 目标激活期间反复点停止，直到目标被清除且空闲；返回期间出现的异常 toast */
async function stopUntilGoalCleared(timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const state = await evalJs(`(() => ({
      active: !!document.querySelector(".send-btn.stop"),
      cleared: !document.querySelector(".goal-clear-btn"),
      toast: document.querySelector(".toast")?.textContent ?? "",
    }))()`);
    if (state.toast.includes("expected active turn")) return state.toast;
    if (!state.active && state.cleared) return "";
    if (state.active) {
      await evalJs(`document.querySelector(".send-btn.stop").click()`);
      await sleep(800);
    } else {
      await sleep(500);
    }
  }
  return "停止后目标未清除";
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

  const goalFile = path.join(testDir, "goal-ok.txt");
  const goalText = `创建文件 ${goalFile}，内容写入 ${MARKER}，完成后停止`;

  // 场景 0：未创建会话时旗子可用，可预填目标并取消预填
  const flagUsable = await evalJs(`(() => {
    const b = document.querySelector(".goal-icon-btn");
    return !!b && !b.disabled;
  })()`);
  record("目标: 未创建会话时旗子可用", flagUsable === true);

  await clickGoalFlag();
  await fillGoalAndConfirm(goalText);
  record("目标: 会话前预填目标生效（× 出现，待应用）", true);

  await cancelGoalViaX();
  record("目标: 预填目标可被 × 直接取消", true);

  // 重新预填真实目标，随后发送首条消息创建会话并自动挂载
  await clickGoalFlag();
  await fillGoalAndConfirm(goalText);
  await setEditorText("开始执行");
  const sent = await clickSend();
  if (!sent) throw new Error("发送失败");
  await waitFor(
    "目标自动挂载（× 保持存在）",
    `!!document.querySelector(".goal-clear-btn")`,
    60000,
  );
  record("目标: 创建会话后自动挂载预填目标", true);

  // 场景 1：服务端围绕目标自动续跑完成 → 旗子颜色状态变为已完成
  await waitFor(
    "目标完成状态（旗子 green class）",
    `!!document.querySelector(".goal-icon-btn.status-completed")`,
    300000,
  );
  const fileOk =
    fs.existsSync(goalFile) &&
    fs.readFileSync(goalFile, "utf8").includes(MARKER);
  record("目标: 实质性目标已完成（文件含标记词）", fileOk);

  // 场景 2：× 取消已挂载目标
  await cancelGoalViaX();
  record("目标: × 取消已挂载目标", true);

  // 场景 3：目标激活期间点停止 → 先清目标再中断
  await clickGoalFlag();
  await fillGoalAndConfirm(
    `分析 ${testDir} 目录下的所有文件并给出架构总结，回复 ${MARKER2}`,
  );
  const stopError = await stopUntilGoalCleared(180000);
  record(
    "目标: 停止按钮先清目标（无 expected active turn 异常）",
    stopError === "",
    stopError.slice(0, 120),
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
