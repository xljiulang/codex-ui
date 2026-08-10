// codex-ui @ 文件 / 插件（技能）与 $ 技能 E2E 验证
// 通过 WebView2 远程调试（CDP 9222）驱动真实 release UI + 少量真实模型调用。
// 用法: node scripts/verify-mentions.mjs
import { spawn, execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = path.join(repoRoot, "src-tauri", "target", "release", "codex-ui.exe");
const CDP_PORT = 9222;
const CDP_BASE = `http://127.0.0.1:${CDP_PORT}`;

const TEST_TAG = "[测试@$]";
const MARKER =
  "MENTION_MARKER_" + crypto.randomBytes(4).toString("hex").toUpperCase();

const testDir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-ui-mention-test-"));
fs.writeFileSync(
  path.join(testDir, "hello.txt"),
  `标记词: ${MARKER}\n这是 codex-ui @/$ 专项测试用的引用文件。\n`,
  "utf8",
);
const evidenceDir = fs.mkdtempSync(
  path.join(os.tmpdir(), "codex-ui-mention-evidence-"),
);

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
      if (fs.readFileSync(f, "utf8").includes(MARKER)) {
        fs.unlinkSync(f);
        removed++;
      }
    } catch {}
  }
  if (removed) log(`已删除 ${removed} 个测试会话文件`);
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
  throw new Error("CDP 连接超时，请确认 9222 端口未被占用且应用已启动");
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
      await sleep(300);
    }
    throw new Error(`等待超时: ${desc}，最后结果: ${JSON.stringify(last)}`);
  };
  client.screenshot = async (name) => {
    try {
      const r = await client.send("Page.captureScreenshot", { format: "png" });
      const p = path.join(evidenceDir, name);
      fs.writeFileSync(p, Buffer.from(r.data, "base64"));
      log(`截图已保存: ${p}`);
    } catch (e) {
      log(`截图失败 ${name}: ${e.message}`);
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

async function connectCdp(url) {
  cdp = await createClient(url);
}

const cdpSend = (method, params = {}) => cdp.send(method, params);
const evalJs = (expression) => cdp.evalJs(expression);
const screenshot = (name) => cdp.screenshot(name);
const waitFor = (desc, expr, timeoutMs) =>
  cdp.waitFor(desc, expr, timeoutMs);

async function setInput(text) {
  await evalJs(`(() => {
    const ed = window.__CODEX_UI_EDITOR__;
    if (!ed) throw new Error("编辑器实例未暴露");
    // 光标移到末尾后插入文本，触发 TipTap onUpdate / selectionUpdate 检测 @ / $ token
    ed.commands.setTextSelection(ed.state.doc.content.size);
    ed.commands.insertContent(${JSON.stringify(text)});
    return ed.getText();
  })()`);
}

async function clearInput() {
  await evalJs(`(() => {
    const ed = window.__CODEX_UI_EDITOR__;
    if (!ed) return;
    ed.commands.setContent("");
  })()`);
}

async function bodyText() {
  return evalJs("document.body.innerText");
}

async function chipLabels() {
  return evalJs(
    `Array.from(document.querySelectorAll(".ref-chip")).map((x) => x.textContent.trim())`,
  );
}

async function hoverChipTooltip(selectorText) {
  return evalJs(`(async () => {
    const bubbles = document.querySelectorAll(".msg-user .bubble");
    const b = bubbles[bubbles.length - 1];
    const chip = Array.from(b.querySelectorAll(".mention-inline")).find((x) =>
      x.textContent.trim().includes(${JSON.stringify(selectorText)}),
    );
    if (!chip) return null;
    chip.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 250));
    const tip = document.querySelector(".ref-tooltip");
    const text = tip ? tip.textContent.trim() : null;
    chip.dispatchEvent(new MouseEvent("mouseleave", { bubbles: true }));
    return text;
  })()`);
}

async function clickMenuButton(text) {
  const r = await evalJs(`(() => {
    const btns = Array.from(document.querySelectorAll(".mention-menu .menu-item"));
    const b = btns.find((x) => x.textContent.includes(${JSON.stringify(text)}));
    if (!b) return { ok: false, labels: btns.map((x) => x.textContent.trim()).slice(0, 15) };
    b.click();
    return { ok: true };
  })()`);
  return r;
}

async function clickSend() {
  return evalJs(`(() => {
    const b = document.querySelector("button.send-btn");
    if (!b) return { ok: false, reason: "send-btn 不存在" };
    if (b.disabled) return { ok: false, reason: "send-btn 禁用" };
    b.click();
    return { ok: true };
  })()`);
}

async function waitTurnDone(timeoutMs = 240000) {
  const deadline = Date.now() + timeoutMs;
  let lastText = "";
  while (Date.now() < deadline) {
    const st = await evalJs(`(() => {
      const stop = document.querySelector(".send-btn.stop");
      const agents = Array.from(document.querySelectorAll(".msg-agent"));
      return {
        active: !!stop,
        last: agents.length ? agents[agents.length - 1].innerText : "",
      };
    })()`);
    if (!st.active && st.last && st.last.length > 0) {
      if (st.last === lastText) return st.last;
      lastText = st.last;
    }
    await sleep(1000);
  }
  throw new Error("回合完成等待超时，最后助手文本: " + lastText.slice(0, 200));
}

async function userBubbleText() {
  return evalJs(`(() => {
    const u = Array.from(document.querySelectorAll(".msg-user"));
    return u.length ? u[u.length - 1].innerText : "";
  })()`);
}

async function selectMention(input, target, kind) {
  await setInput(input);
  await waitFor(
    `${kind} 菜单出现`,
    `document.querySelector(".mention-menu") ? true : false`,
    10000,
  );
  await waitFor(
    `菜单项 ${target} 出现`,
    `(() => {
      const btns = Array.from(document.querySelectorAll(".mention-menu .menu-item"));
      return btns.some((x) => x.textContent.includes(${JSON.stringify(target)}));
    })()`,
    20000,
  );
  const r = await clickMenuButton(target);
  if (!r.ok) throw new Error(`点击菜单项失败: ${JSON.stringify(r.labels)}`);
  await sleep(300);
}

async function selectPluginByEnter(input, target) {
  await setInput(input);
  await waitFor(
    `@ 菜单出现`,
    `document.querySelector(".mention-menu") ? true : false`,
    10000,
  );
  await waitFor(
    `插件项 ${target} 出现`,
    `(() => {
      const btns = Array.from(document.querySelectorAll(".mention-menu .menu-item"));
      return btns.some((x) => x.textContent.includes(${JSON.stringify(target)}));
    })()`,
    20000,
  );
  // 固定行（选择文件/文件夹）在前，用 ↓ 把高亮移动到目标插件后 Enter 选中（不发送）
  await evalJs(`(async () => {
    const el = document.querySelector(".ProseMirror");
    for (let i = 0; i < 12; i++) {
      const active = document.querySelector(".mention-menu .menu-item.active");
      if (active && active.textContent.includes(${JSON.stringify(target)})) break;
      el.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      await new Promise((r) => setTimeout(r, 60));
    }
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  })()`);
  await sleep(300);
}

async function selectSkillByEnter(input, target) {
  await setInput(input);
  await waitFor(
    `$ 菜单出现`,
    `document.querySelector(".mention-menu") ? true : false`,
    10000,
  );
  await waitFor(
    `技能项 ${target} 出现`,
    `(() => {
      const btns = Array.from(document.querySelectorAll(".mention-menu .menu-item"));
      return btns.some((x) => x.textContent.includes(${JSON.stringify(target)}));
    })()`,
    20000,
  );
  // $ 菜单首项即高亮技能，Enter 选中（不再直接发送）
  await evalJs(`(() => {
    const el = document.querySelector(".ProseMirror");
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  })()`);
  await sleep(300);
}

async function main() {
  if (!fs.existsSync(APP)) throw new Error(`未找到应用: ${APP}`);
  log(`测试目录: ${testDir}`);
  log(`标记词: ${MARKER}`);

  child = spawn(APP, [], {
    cwd: testDir,
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
    },
    stdio: "ignore",
    // 必须可见启动：WindowStyle Hidden / windowsHide 会阻止 WebView2 初始化，
    // 导致远程调试端口不开放。
    windowsHide: false,
  });
  log("应用已启动，等待 CDP…");

  const page = await waitForCdp();
  const mainPageId = page.id;
  await connectCdp(page.webSocketDebuggerUrl);
  await waitFor(
    "编辑器加载",
    `!!document.querySelector(".ProseMirror")`,
    60000,
  );
  log("UI 就绪");

  // ---------- 场景 1: @ 单独 ----------
  log("场景 1: @ 单独");
  await selectMention("@hello", "hello.txt", "@");
  const chip1 = await chipLabels();
  record(
    "@ 单独: 选中后出现 @hello.txt 附件标签且触发词被移除",
    chip1.some((l) => l.includes("@hello.txt")),
    chip1.some((l) => l.includes("@hello.txt")) ? "" : "附件标签: " + JSON.stringify(chip1),
  );
  await screenshot("1-at-chip.png");

  await setInput(`${TEST_TAG} 读取你被引用的文件，只回答文件里的标记词，不要解释。`);
  const send1 = await clickSend();
  if (!send1.ok) throw new Error("场景1 发送失败: " + send1.reason);
  const reply1 = await waitTurnDone();
  const bubble1 = await userBubbleText();
  record(
    "@ 单独: 回显渲染 @hello.txt 标签",
    bubble1.includes("@hello.txt"),
    bubble1.includes("@hello.txt") ? "" : "用户气泡未见 @hello.txt",
  );
  const fileTip = await hoverChipTooltip("@hello.txt");
  record(
    "悬浮: 文件 chip 悬浮显示路径",
    !!fileTip && fileTip.includes("hello.txt") && fileTip.includes("文件"),
    JSON.stringify(fileTip),
  );
  record(
    "@ 单独: 模型回复包含文件标记词（文件可被读取）",
    reply1.includes(MARKER),
    reply1.slice(0, 200),
  );
  await screenshot("1-at-reply.png");

  // ---------- 场景 2: $ 单独 ----------
  log("场景 2: $ 单独");
  const msgCountBefore = await evalJs(
    `document.querySelectorAll(".msg-user").length`,
  );
  await selectSkillByEnter("$csharp-code-rules", "csharp-code-rules");
  const chip2 = await chipLabels();
  record(
    "$ 单独: 选中后出现 $csharp-code-rules 附件标签且触发词被移除",
    chip2.some((l) => l.includes("$csharp-code-rules")),
    chip2.some((l) => l.includes("$csharp-code-rules"))
      ? ""
      : "附件标签: " + JSON.stringify(chip2),
  );
  await sleep(1500);
  const msgCountAfter = await evalJs(
    `document.querySelectorAll(".msg-user").length`,
  );
  record(
    "$ 单独: Enter 选中技能而未发送消息",
    msgCountAfter === msgCountBefore,
    `消息数 ${msgCountBefore} -> ${msgCountAfter}`,
  );
  await screenshot("2-skill-chip.png");

  await setInput(
    `${TEST_TAG} 只回答：注入给你的技能的名称，以及它的第一条规则的一句话要点，不要展开。`,
  );
  const send2 = await clickSend();
  if (!send2.ok) throw new Error("场景2 发送失败: " + send2.reason);
  const reply2 = await waitTurnDone();
  const bubble2 = await userBubbleText();
  record(
    "$ 单独: 回显渲染 $csharp-code-rules 标签",
    bubble2.includes("$csharp-code-rules"),
    bubble2.includes("$csharp-code-rules")
      ? ""
      : "用户气泡未见 $csharp-code-rules",
  );
  const skillTip = await hoverChipTooltip("$csharp-code-rules");
  record(
    "悬浮: 技能 chip 悬浮显示说明",
    !!skillTip && skillTip.includes("C#") && skillTip.includes("技能"),
    JSON.stringify(skillTip),
  );
  record(
    "$ 单独: 模型回复体现技能内容（SKILL.md 注入生效）",
    reply2.toLowerCase().includes("csharp-code-rules"),
    reply2.slice(0, 200),
  );
  await screenshot("2-skill-reply.png");

  // ---------- 场景 2b: @ 联合搜索选择插件 ----------
  log("场景 2b: @ 联合搜索选择插件");
  const msgCountBeforeB = await evalJs(
    `document.querySelectorAll(".msg-user").length`,
  );
  await selectPluginByEnter("@doc", "Documents");
  const chip2b = await chipLabels();
  record(
    "插件: 选中后出现 @documents 内联 chip 且触发词被移除",
    chip2b.some((l) => l.includes("@documents")),
    chip2b.some((l) => l.includes("@documents"))
      ? ""
      : "内联 chip: " + JSON.stringify(chip2b),
  );
  await sleep(1500);
  const msgCountAfterB = await evalJs(
    `document.querySelectorAll(".msg-user").length`,
  );
  record(
    "插件: Enter 选中插件而未发送消息",
    msgCountAfterB === msgCountBeforeB,
    `消息数 ${msgCountBeforeB} -> ${msgCountAfterB}`,
  );
  await screenshot("2b-plugin-chip.png");

  await setInput(
    `${TEST_TAG} 只回答：注入给你的技能的名称，以及它的第一条规则的一句话要点，不要展开。`,
  );
  const send2b = await clickSend();
  if (!send2b.ok) throw new Error("场景2b 发送失败: " + send2b.reason);
  const reply2b = await waitTurnDone();
  const bubble2b = await userBubbleText();
  record(
    "插件: 回显渲染 @documents 标签（插件 @ 前缀）",
    bubble2b.includes("@documents"),
    bubble2b.includes("@documents") ? "" : "用户气泡未见 @documents",
  );
  const pluginTip = await hoverChipTooltip("@documents");
  record(
    "悬浮: 插件 chip 悬浮显示说明",
    !!pluginTip &&
      pluginTip.includes("document") &&
      !pluginTip.startsWith("plugin://") &&
      pluginTip.includes("插件"),
    JSON.stringify(pluginTip),
  );
  await evalJs(`(async () => {
    const bubbles = document.querySelectorAll(".msg-user .bubble");
    const b = bubbles[bubbles.length - 1];
    const chip = Array.from(b.querySelectorAll(".mention-inline")).find((x) =>
      x.textContent.trim().includes("@documents"),
    );
    if (chip) chip.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 250));
  })()`);
  await screenshot("2b-plugin-tooltip.png");
  await evalJs(`(() => {
    const bubbles = document.querySelectorAll(".msg-user .bubble");
    const b = bubbles[bubbles.length - 1];
    const chip = Array.from(b.querySelectorAll(".mention-inline")).find((x) =>
      x.textContent.trim().includes("@documents"),
    );
    if (chip) chip.dispatchEvent(new MouseEvent("mouseleave", { bubbles: true }));
  })()`);
  const pluginChipClickable = await evalJs(`(() => {
    const bubbles = document.querySelectorAll(".msg-user .bubble");
    const b = bubbles[bubbles.length - 1];
    const chip = Array.from(b.querySelectorAll(".mention-inline")).find((x) =>
      x.textContent.trim().includes("@documents"),
    );
    return chip ? chip.classList.contains("clickable") : null;
  })()`);
  record(
    "插件: @documents chip 不可点击（plugin:// 无本地路径）",
    pluginChipClickable === false,
    JSON.stringify(pluginChipClickable),
  );
  record(
    "插件: 模型回复体现技能内容（SKILL.md 注入生效）",
    reply2b.toLowerCase().includes("documents"),
    reply2b.slice(0, 200),
  );
  await screenshot("2b-plugin-reply.png");

  // ---------- 场景 3: 混编顺序（文本-文件-文本-技能-文本） ----------
  log("场景 3: 混编顺序");
  await clearInput();
  await setInput("先 ");
  await selectMention("@hello", "hello.txt", "@");
  await setInput("中间 ");
  await selectMention("$csharp-code-rules", "csharp-code-rules", "$");
  await setInput(
    `结尾 ${TEST_TAG} 两件事：1) 引用文件里的标记词；2) 注入技能的名称。只列这两项。`,
  );
  const chip3 = await chipLabels();
  record(
    "混编: 编辑器内 chip 顺序为 @hello.txt → $csharp-code-rules",
    chip3.indexOf("@hello.txt") >= 0 &&
      chip3.indexOf("$csharp-code-rules") > chip3.indexOf("@hello.txt"),
    chip3.length === 0
      ? ""
      : "内联 chip: " + JSON.stringify(chip3),
  );
  await screenshot("3-mixed-chips.png");

  const send3 = await clickSend();
  if (!send3.ok) throw new Error("场景3 发送失败: " + send3.reason);
  const bubble3 = await userBubbleText();
  record(
    "混编: 回显顺序与输入一致（先 @hello.txt 中间 $csharp-code-rules 结尾）",
    bubble3.indexOf("先") < bubble3.indexOf("@hello.txt") &&
      bubble3.indexOf("@hello.txt") < bubble3.indexOf("中间") &&
      bubble3.indexOf("中间") < bubble3.indexOf("$csharp-code-rules") &&
      bubble3.indexOf("$csharp-code-rules") < bubble3.indexOf("结尾"),
    bubble3.slice(0, 160),
  );
  record(
    "混编: 回显同时渲染 @hello.txt 与 $csharp-code-rules",
    bubble3.includes("@hello.txt") && bubble3.includes("$csharp-code-rules"),
    bubble3.slice(0, 160),
  );
  const inlineLayout = await evalJs(`(() => {
    const bubbles = document.querySelectorAll(".msg-user .bubble");
    const b = bubbles[bubbles.length - 1];
    if (!b) return { ok: false, reason: "no bubble" };
    const direct = Array.from(b.children);
    return {
      ok:
        !direct.some((el) => el.classList.contains("md") && !el.classList.contains("md-inline")) &&
        b.querySelectorAll(":scope > .md-inline").length === 3,
      blocks: direct.filter((el) => el.classList.contains("md") && !el.classList.contains("md-inline")).length,
      inlineCount: b.querySelectorAll(":scope > .md-inline").length,
      chipCount: b.querySelectorAll(":scope > .mention-inline").length,
    };
  })()`);
  record("混编: 回显文本片段为内联盒子（无块级 .md，保持单行）", inlineLayout.ok, JSON.stringify(inlineLayout));
  // 点击回显文件 chip → reveal_path 定位（测试钩子只记录不真开资源管理器）
  await evalJs(`(() => {
    window.__CODEX_UI_TEST__ = true;
    window.__CODEX_UI_TEST_LOG__ = [];
    const bubbles = document.querySelectorAll(".msg-user .bubble");
    const b = bubbles[bubbles.length - 1];
    const chip = Array.from(b.querySelectorAll(".mention-inline.clickable")).find((x) =>
      x.textContent.includes("@hello.txt"),
    );
    if (chip) chip.click();
  })()`);
  await sleep(400);
  const fileChipLog = await evalJs(`window.__CODEX_UI_TEST_LOG__`);
  record(
    "混编: 点击回显文件 chip 触发 reveal_path 定位文件",
    fileChipLog.some(
      (l) => l.cmd === "reveal_path" && l.args?.path === testDir + "\\hello.txt",
    ),
    JSON.stringify(fileChipLog),
  );
  await evalJs(`window.__CODEX_UI_TEST__ = false;`);
  const reply3 = await waitTurnDone();
  record(
    "混编: 模型回复同时覆盖文件标记词与技能名称",
    reply3.includes(MARKER) && reply3.toLowerCase().includes("csharp-code-rules"),
    reply3.slice(0, 200),
  );
  await screenshot("3-mixed-reply.png");

  // ---------- 场景 3b: 用户消息 Markdown 渲染 ----------
  log("场景 3b: 用户消息 Markdown 渲染");
  await setInput(`${TEST_TAG}\n# 测试标题\n\n**加粗内容** 只回复"好的"`);
  const send3b = await clickSend();
  if (!send3b.ok) throw new Error("场景3b 发送失败: " + send3b.reason);
  await waitTurnDone();
  const userMd = await evalJs(`(() => {
    const users = Array.from(document.querySelectorAll(".msg-user"));
    const last = users[users.length - 1];
    return {
      h1: last?.querySelector(".md h1")?.textContent ?? "",
      strong: last?.querySelector(".md strong")?.textContent ?? "",
    };
  })()`);
  record(
    "用户消息 Markdown 渲染",
    userMd.h1 === "测试标题" && userMd.strong === "加粗内容",
    JSON.stringify(userMd),
  );
  await screenshot("3b-user-md.png");

  // ---------- 场景 4: 渲染抽查（代码块 pre/高亮/复制按钮/语言徽标） ----------
  log("场景 4: 渲染抽查");
  await setInput(
    `${TEST_TAG} 只回复一个包含 const answer = 42 的 javascript 代码块，不要其它文字。`,
  );
  const send4 = await clickSend();
  if (!send4.ok) throw new Error("场景4 发送失败: " + send4.reason);
  await waitTurnDone();
  const codeBlock = await evalJs(`(() => {
    const codes = Array.from(document.querySelectorAll(".msg-agent pre code.hljs"));
    const copyBtn = !!document.querySelector(".msg-agent .code-copy-btn");
    const langChip = Array.from(
      document.querySelectorAll(".msg-agent .code-lang"),
    ).map((x) => x.textContent.trim());
    return { highlighted: codes.length > 0, copyBtn, langChip };
  })()`);
  record(
    "渲染: 代码块保留 pre 且有语法高亮",
    codeBlock.highlighted,
    JSON.stringify(codeBlock),
  );
  record(
    "渲染: 代码块有复制按钮与语言徽标",
    codeBlock.copyBtn &&
      codeBlock.langChip.some((l) => ["js", "javascript"].includes(l)),
    JSON.stringify(codeBlock),
  );
  await screenshot("6-codeblock.png");

  // ---------- 场景 5: 链接分发抽查（拦截 invoke，避免真实弹出浏览器/资源管理器） ----------
  log("场景 5: 链接分发抽查");
  const localLink =
    "file:///" + testDir.replace(/\\/g, "/") + "/hello.txt";
  const driveLink = testDir.replace(/\\/g, "/") + "/hello.txt";
  await setInput(
    `${TEST_TAG} 只回复一行 Markdown 链接文本：[网页](https://example.com)，不要代码块、不要其它内容。`,
  );
  const send5a = await clickSend();
  if (!send5a.ok) throw new Error("场景5a 发送失败: " + send5a.reason);
  await waitTurnDone();
  await setInput(
    `${TEST_TAG} 只回复一行 Markdown 链接文本：[hello.txt](${localLink})，不要代码块、不要其它内容。`,
  );
  const send5b = await clickSend();
  if (!send5b.ok) throw new Error("场景5b 发送失败: " + send5b.reason);
  await waitTurnDone();
  await setInput(
    `${TEST_TAG} 只回复一行 Markdown 链接文本：[hello.txt](${driveLink})，不要代码块、不要其它内容。`,
  );
  const send5c = await clickSend();
  if (!send5c.ok) throw new Error("场景5c 发送失败: " + send5c.reason);
  await waitTurnDone();

  const linkFound = await evalJs(`(() => {
    const links = Array.from(document.querySelectorAll(".msg-agent .md a"));
    return {
      web: links.some((a) => a.getAttribute("href")?.startsWith("https://")),
      local: links.some((a) => a.getAttribute("href")?.startsWith("file://")),
      drive: links.some((a) => a.getAttribute("href")?.startsWith("C:")),
      ready: links.every((a) => a.getAttribute("data-link-ready") === "1"),
    };
  })()`);
  record(
    "链接: 回复中包含网页与本地 file:// 链接",
    linkFound.web && linkFound.local && linkFound.ready,
    JSON.stringify(linkFound),
  );
  record(
    "链接: 盘符路径链接保留 href（不再被 DOMPurify 剥离）",
    linkFound.drive,
    JSON.stringify(linkFound),
  );
  // 通过应用内置测试钩子记录 openLink 分发（__TAURI_INTERNALS__.invoke 不可配置，无法拦截）
  await evalJs(`(() => {
    window.__CODEX_UI_TEST__ = true;
    window.__CODEX_UI_TEST_LOG__ = [];
    const links = Array.from(document.querySelectorAll(".msg-agent .md a"));
    links.filter((a) => a.getAttribute("href")?.startsWith("https://")).forEach((a) => a.click());
    links.filter((a) => a.getAttribute("href")?.startsWith("file://")).forEach((a) => a.click());
    links.filter((a) => a.getAttribute("href")?.startsWith("C:")).forEach((a) => a.click());
  })()`);
  await sleep(400);
  const invokeLog = await evalJs(`window.__CODEX_UI_TEST_LOG__`);
  record(
    "链接: 网页链接调用 open_url",
    invokeLog.some(
      (l) => l.cmd === "open_url" && l.args?.url === "https://example.com",
    ),
    JSON.stringify(invokeLog),
  );
  record(
    "链接: 本地链接调用 reveal_path 定位文件",
    invokeLog.some(
      (l) =>
        l.cmd === "reveal_path" &&
        l.args?.path === testDir + "\\hello.txt",
    ),
    JSON.stringify(invokeLog),
  );
  record(
    "链接: file:// 与盘符路径两种本地形式均分发 reveal_path",
    invokeLog.filter((l) => l.cmd === "reveal_path").length >= 2,
    JSON.stringify(invokeLog),
  );
  await screenshot("7-links.png");
  await evalJs(`window.__CODEX_UI_TEST__ = false;`);

  // ---------- 场景 6: 文件变更完整 diff 预览（真实 apply_patch，多文件） ----------
  log("场景 6: 文件变更 diff 预览");
  await setInput(
    `${TEST_TAG} 请用 apply_patch 工具：1) 修改文件 ${testDir}\\hello.txt，把第一行"标记词: ${MARKER}"改为"标记词已修改: ${MARKER}"；2) 新建文件 ${testDir}\\sample.ts，内容为一行 "const answer: number = 42;"。不要用其它工具。`,
  );
  const send6 = await clickSend();
  if (!send6.ok) throw new Error("场景6 发送失败: " + send6.reason);
  await waitTurnDone();
  const rowsFound = await evalJs(
    `Array.from(document.querySelectorAll(".tool-card .change-row.clickable")).map((r) => r.textContent.trim())`,
  );
  record(
    "文件变更: 出现可点击的变更行",
    rowsFound.length > 0,
    JSON.stringify(rowsFound),
  );

  async function openDiffWindowAndWait(targetText) {
    await evalJs(
      `(() => {
        const rows = Array.from(document.querySelectorAll(".tool-card .change-row.clickable"));
        const row = rows.find((r) => r.textContent.includes(${JSON.stringify(
          targetText,
        )}));
        if (row) row.click();
      })()`,
    );
    let diffTarget = null;
    for (let i = 0; i < 40; i++) {
      const list = await (await fetch(`${CDP_BASE}/json/list`)).json();
      diffTarget = list.find(
        (t) => t.type === "page" && t.id !== mainPageId,
      );
      if (diffTarget) break;
      await sleep(500);
    }
    if (!diffTarget) return null;
    const d = await createClient(diffTarget.webSocketDebuggerUrl);
    // 复用窗口时 target id 不变，等待内容切到目标文件
    await d.waitFor(
      "diff 窗口内容就绪",
      `(() => {
        const path = document.querySelector(".diff-window-path")?.textContent ?? "";
        if (!path.includes(${JSON.stringify(targetText)})) return false;
        const loading = !!document.querySelector(".diff-loading");
        const rows = document.querySelectorAll(".diff-row").length;
        return !loading && (rows > 0 || !!document.querySelector(".diff-fallback-note"));
      })()`,
      15000,
    );
    return d;
  }

  async function checkNoContextMenu(d) {
    return d.evalJs(`(() => {
      const el = document.querySelector(".diff-text");
      if (!el) return { ok: false, reason: "no diff-text" };
      const ev = new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: 60,
        clientY: 60,
      });
      el.dispatchEvent(
        ev,
      );
      return new Promise((r) =>
        setTimeout(() => {
          r({
            ok: ev.defaultPrevented && !document.querySelector(".ctx-menu"),
          });
        }, 150),
      );
    })()`);
  }

  if (rowsFound.length > 0) {
    // hello.txt：旧/新/分隔 + 右键无菜单
    const d1 = await openDiffWindowAndWait("hello.txt");
    record("文件变更: hello.txt diff 窗口打开", !!d1);
    if (d1) {
      const inline = await d1.evalJs(`(() => {
        const rows = Array.from(document.querySelectorAll(".diff-row"));
        return {
          hasOld: rows.some(
            (r) => r.classList.contains("del") && r.textContent.includes("标记词:"),
          ),
          hasNew: rows.some(
            (r) =>
              r.classList.contains("add") &&
              r.textContent.includes("标记词已修改"),
          ),
          hasSep: rows.some(
            (r) => r.classList.contains("sep") && r.textContent.includes("旧 | 新"),
          ),
        };
      })()`);
      record(
        "文件变更: 旧行在上、新行在下并含“旧 | 新”分隔",
        inline.hasOld && inline.hasNew && inline.hasSep,
        JSON.stringify(inline),
      );
      const ctx1 = await checkNoContextMenu(d1);
      record("文件变更: 右键无菜单（阻止默认）", ctx1.ok, JSON.stringify(ctx1));
      await d1.screenshot("8-diff-preview.png");
      d1.close();
    }

    // sample.ts：代码语法高亮
    const d2 = await openDiffWindowAndWait("sample.ts");
    record("文件变更: sample.ts diff 窗口打开（语法高亮）", !!d2);
    if (d2) {
      const hl = await d2.evalJs(`(() => {
        const kw = document.querySelectorAll(".diff-text .hljs-keyword").length;
        return {
          kw,
          has: kw > 0,
          text: document.querySelector(".diff-text")?.textContent ?? "",
          path: document.querySelector(".diff-window-path")?.textContent ?? "",
        };
      })()`);
      record("文件变更: 代码文件语法高亮", hl.has, JSON.stringify(hl));
      const ctx2 = await checkNoContextMenu(d2);
      record(
        "文件变更: 代码窗口右键无菜单",
        ctx2.ok,
        JSON.stringify(ctx2),
      );
      await d2.screenshot("9-diff-highlight.png");
      d2.close();
    }
  }

  // ---------- UI 边界抽查（不发真实模型） ----------
  log("UI 边界抽查");
  await setInput("$ida-pro-mcp:idapython");
  await waitFor("冒号技能名菜单", `!!document.querySelector(".mention-menu")`, 10000);
  await waitFor(
    "冒号技能名按钮出现",
    `(() => {
      const btns = Array.from(document.querySelectorAll(".mention-menu .menu-item"));
      return btns.some((x) => x.textContent.includes("ida-pro-mcp:idapython"));
    })()`,
    15000,
  );
  record("边界: $ 技能名含冒号可整名触发菜单并命中", true);
  await screenshot("5-dollar-colon.png");
  await evalJs(`window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }))`);
  await sleep(300);

  await clearInput();
  await setInput("@ida");
  await waitFor("插件搜索菜单", `!!document.querySelector(".mention-menu")`, 10000);
  await waitFor(
    "插件项出现",
    `(() => {
      const btns = Array.from(document.querySelectorAll(".mention-menu .menu-item"));
      return btns.some((x) => x.textContent.includes("IDA Pro MCP"));
    })()`,
    15000,
  );
  const pluginSearch = true;
  record("边界: @ 联合搜索命中无图标插件（品牌色首字母占位）", pluginSearch);
  await screenshot("5-plugin-search.png");
  await evalJs(`window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }))`);
  await sleep(300);

  await clearInput();
  await setInput("看下@file");
  await sleep(800);
  const noMenu = await evalJs(`!document.querySelector(".mention-menu")`);
  record("边界: 词中 @（前面无空格）不弹菜单", noMenu);

  await clearInput();
  await setInput("@");
  await waitFor("@ 空 token 菜单", `!!document.querySelector(".mention-menu")`, 5000);
  const emptyRows = await evalJs(`(() => {
    const labels = Array.from(
      document.querySelectorAll(".mention-menu .menu-item .menu-item-label"),
    ).map((x) => x.textContent.trim());
    const first = labels.indexOf("选择文件…");
    const second = labels.indexOf("选择文件夹…");
    const plugin = labels.findIndex((l) => l === "Documents");
    return (
      first >= 0 &&
      second > first &&
      plugin > second &&
      !document.querySelector(".plus-btn")
    );
  })()`);
  record("边界: @ 空 token 固定行在前、插件在后，且 + 按钮已移除", emptyRows);
  await screenshot("4-at-empty.png");

  await evalJs(`window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }))`);
  await sleep(300);
  const closed = await evalJs(`!document.querySelector(".mention-menu")`);
  record("边界: Esc 关闭菜单", closed);

  await clearInput();
  await selectMention("@hello", "hello.txt", "@");
  const sendEnabled = await evalJs(`(() => {
    const b = document.querySelector("button.send-btn");
    return b ? !b.disabled : false;
  })()`);
  record("边界: 仅附件无文本时发送按钮可用", sendEnabled);
  // 内联 chip 移除：定位 chip 节点，选中后用 deleteSelection 删除
  const removed = await evalJs(`(() => {
    const ed = window.__CODEX_UI_EDITOR__;
    if (!ed) return false;
    let chipPos = -1;
    let chipSize = 0;
    ed.state.doc.descendants((node, pos) => {
      if (node.type.name === "reference") {
        chipPos = pos;
        chipSize = node.nodeSize;
        return false;
      }
      return true;
    });
    if (chipPos < 0) return false;
    ed.chain()
      .setTextSelection({ from: chipPos, to: chipPos + chipSize })
      .deleteSelection()
      .run();
    return document.querySelectorAll(".ref-chip").length === 0;
  })()`);
  record("边界: 内联引用 chip 可移除", removed);

  // ---------- 汇总 ----------
  const pass = results.filter((r) => r.ok).length;
  log(`\n===== 结果汇总: ${pass}/${results.length} 通过 =====`);
  for (const r of results) {
    log(`${r.ok ? "PASS" : "FAIL"} ${r.name}`);
  }
  log(`证据目录: ${evidenceDir}`);
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
