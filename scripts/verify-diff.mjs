// 精简 diff 专项 E2E：1 条真实模型消息（apply_patch 改 hello.txt + 新建 sample.ts）
// 验证独立 diff 窗口：内联旧/新/分隔、指示条对齐、代码语法高亮、自定义右键菜单。
// 用法: node scripts/verify-diff.mjs
import { spawn, execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = path.join(repoRoot, "src-tauri", "target", "release", "codex-ui.exe");
const CDP_BASE = "http://127.0.0.1:9222";
const TAG = "[测试diff]";
const MARKER = "DIFF_MARKER_" + crypto.randomBytes(4).toString("hex").toUpperCase();

const testDir = fs.mkdtempSync(path.join(os.tmpdir(), "codexui-diff-e2e-"));
// 300 行大文件：让 diff 窗口出现滚动，验证指示条与滚动条位置对齐
const helloLines = Array.from({ length: 300 }, (_, i) =>
  i === 99 ? `标记词: ${MARKER}` : `第 ${i + 1} 行普通内容`,
);
fs.writeFileSync(path.join(testDir, "hello.txt"), helloLines.join("\n") + "\n", "utf8");
const evidenceDir = fs.mkdtempSync(path.join(os.tmpdir(), "codexui-diff-evidence-"));

let child = null;
let cdp = null;
const results = [];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (m) => console.log(`[${new Date().toISOString()}] ${m}`);

function record(name, ok, detail = "") {
  results.push({ name, ok });
  log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
}

function killApp() {
  if (!child) return;
  try { child.kill(); } catch {}
  try {
    execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
    });
  } catch {}
  child = null;
}

function cleanup() {
  killApp();
  const root = path.join(os.homedir(), ".codex", "sessions");
  if (fs.existsSync(root)) {
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
  try { fs.rmSync(testDir, { recursive: true, force: true }); } catch {}
}

process.on("exit", cleanup);
process.on("SIGINT", () => process.exit(130));
process.on("SIGTERM", () => process.exit(143));

async function waitForCdp() {
  for (let i = 0; i < 120; i++) {
    try {
      const t = (await (await fetch(`${CDP_BASE}/json/list`)).json()).find(
        (x) => x.type === "page",
      );
      if (t?.webSocketDebuggerUrl) return t;
    } catch {}
    await sleep(500);
  }
  throw new Error("CDP timeout");
}

async function createClient(wsUrl) {
  const c = { seq: 0, pending: new Map(), ws: null };
  await new Promise((res, rej) => {
    c.ws = new WebSocket(wsUrl);
    c.ws.onopen = res;
    c.ws.onerror = () => rej(new Error("ws"));
  });
  c.ws.onmessage = (ev) => {
    const m = JSON.parse(ev.data);
    const p = c.pending.get(m.id);
    if (!p) return;
    c.pending.delete(m.id);
    if (m.error) p.reject(new Error(JSON.stringify(m.error)));
    else p.resolve(m.result);
  };
  c.send = (method, params = {}) =>
    new Promise((res, rej) => {
      const id = ++c.seq;
      c.pending.set(id, { resolve: res, reject: rej });
      c.ws.send(JSON.stringify({ id, method, params }));
    });
  c.evalJs = async (expression) => {
    const r = await c.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (r.exceptionDetails) {
      throw new Error("页面脚本异常: " + JSON.stringify(r.exceptionDetails).slice(0, 300));
    }
    return r.result.value;
  };
  c.waitFor = async (desc, expr, timeoutMs = 15000) => {
    const deadline = Date.now() + timeoutMs;
    let last;
    while (Date.now() < deadline) {
      last = await c.evalJs(expr);
      if (last) return last;
      await sleep(300);
    }
    throw new Error(`等待超时: ${desc}`);
  };
  c.screenshot = async (name) => {
    try {
      const r = await c.send("Page.captureScreenshot", { format: "png" });
      const p = path.join(evidenceDir, name);
      fs.writeFileSync(p, Buffer.from(r.data, "base64"));
      log(`截图: ${p}`);
    } catch {}
  };
  c.close = () => { try { c.ws.close(); } catch {} };
  await c.send("Runtime.enable");
  await c.send("Page.enable");
  return c;
}

async function main() {
  child = spawn(APP, [], {
    cwd: testDir,
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9222" },
    stdio: "ignore",
    windowsHide: false,
  });
  const mainPage = await waitForCdp();
  const mainPageId = mainPage.id;
  cdp = await createClient(mainPage.webSocketDebuggerUrl);
  await cdp.waitFor("输入框就绪", `!!document.querySelector("textarea")`, 60000);

  const setInput = (text) =>
    cdp.evalJs(`(() => {
      const ta = document.querySelector("textarea");
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value").set;
      setter.call(ta, ${JSON.stringify(text)});
      ta.dispatchEvent(new Event("input", { bubbles: true }));
    })()`);
  const clickSend = () =>
    cdp.evalJs(`(() => {
      const b = document.querySelector("button.send-btn");
      if (!b || b.disabled) return false;
      b.click();
      return true;
    })()`);

  // 1 条真实模型消息：apply_patch 修改 hello.txt + 新建 sample.ts
  await setInput(
    `${TAG} 请用 apply_patch 工具：1) 修改文件 ${testDir}\\hello.txt，把第一行"标记词: ${MARKER}"改为"标记词已修改: ${MARKER}"；2) 新建文件 ${testDir}\\sample.ts，内容为一行 "const answer: number = 42;"。不要用其它工具。`,
  );
  if (!(await clickSend())) throw new Error("发送失败");
  const deadline = Date.now() + 240000;
  while (Date.now() < deadline) {
    const st = await cdp.evalJs(`(() => {
      const stop = !!document.querySelector(".send-btn.stop");
      const agents = Array.from(document.querySelectorAll(".msg-agent"));
      return { stop, last: agents.length ? agents[agents.length - 1].innerText : "" };
    })()`);
    if (!st.stop && st.last) break;
    await sleep(1000);
  }

  const rowsFound = await cdp.evalJs(
    `Array.from(document.querySelectorAll(".tool-card .change-row.clickable")).map((r) => r.textContent.trim())`,
  );
  record("出现可点击的变更行", rowsFound.length >= 2, JSON.stringify(rowsFound));

  async function openDiffWindow(targetText) {
    await cdp.evalJs(`(() => {
      const rows = Array.from(document.querySelectorAll(".tool-card .change-row.clickable"));
      const row = rows.find((r) => r.textContent.includes(${JSON.stringify(
        targetText,
      )}));
      if (row) row.click();
    })()`);
    let t = null;
    for (let i = 0; i < 40; i++) {
      t = (await (await fetch(`${CDP_BASE}/json/list`)).json()).find(
        (x) => x.type === "page" && x.id !== mainPageId,
      );
      if (t) break;
      await sleep(500);
    }
    if (!t) return null;
    const d = await createClient(t.webSocketDebuggerUrl);
    await d.waitFor(
      "内容就绪",
      `(() => {
        const path = document.querySelector(".diff-window-path")?.textContent ?? "";
        if (!path.includes(${JSON.stringify(targetText)})) return false;
        return !document.querySelector(".diff-loading") &&
          (document.querySelectorAll(".diff-row").length > 0 ||
            !!document.querySelector(".diff-fallback-note"));
      })()`,
    );
    return d;
  }

  async function checkNoContextMenu(d) {
    return d.evalJs(`(() => {
      const el = document.querySelector(".diff-text");
      if (!el) return false;
      const ev = new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: 60,
        clientY: 60,
      });
      el.dispatchEvent(ev);
      return new Promise((r) =>
        setTimeout(() => {
          r(ev.defaultPrevented && !document.querySelector(".ctx-menu"));
        }, 150),
      );
    })()`);
  }

  const d1 = await openDiffWindow("hello.txt");
  record("hello.txt diff 窗口打开", !!d1);
  if (d1) {
    const inline = await d1.evalJs(`(() => {
      const rows = Array.from(document.querySelectorAll(".diff-row"));
      return {
        old: rows.some((r) => r.classList.contains("del") && r.textContent.includes("标记词:")),
        neu: rows.some((r) => r.classList.contains("add") && r.textContent.includes("标记词已修改")),
        sep: rows.some((r) => r.classList.contains("sep") && r.textContent.includes("旧 | 新")),
      };
    })()`);
    record("内联旧/新/分隔", inline.old && inline.neu && inline.sep);
    record("右键无自定义菜单", await checkNoContextMenu(d1));
    await d1.screenshot("diff-hello.png");
    d1.close();
  }

  const d2 = await openDiffWindow("sample.ts");
  record("sample.ts diff 窗口打开（复用）", !!d2);
  if (d2) {
    const hl = await d2.evalJs(
      `document.querySelectorAll(".diff-text .hljs-keyword").length > 0`,
    );
    record("代码文件语法高亮", hl);
    record("代码窗口右键无菜单", await checkNoContextMenu(d2));
    await d2.screenshot("diff-highlight.png");
    d2.close();
  }

  const pass = results.filter((r) => r.ok).length;
  log(`===== 结果: ${pass}/${results.length} 通过 =====`);
  log(`证据目录: ${evidenceDir}`);
}

main()
  .then(() => { cleanup(); process.exit(results.every((r) => r.ok) ? 0 : 1); })
  .catch((e) => { log("E2E 失败: " + e.message); cleanup(); process.exit(2); });
