// codex-ui 三主题样式审计：CDP 截图取证（模式与 verify-*.mjs 一致）
// 用法:
//   node scripts/audit-themes.mjs --phase static [--port 9225] [--out <dir>]
//   node scripts/audit-themes.mjs --phase chat   [--port 9225] [--out <dir>]
// static: 空状态×3 + 设置弹窗×3 + 历史面板×1 + 令牌快照
// chat:   真实回合（Markdown 富内容 + 命令工具卡片）×3 主题
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  createClient,
  killAppTree,
  sleep,
  spawnApp,
  waitForEditor,
} from "./lib/e2e.mjs";

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const PHASE = arg("phase", "static");
const CDP_PORT = Number(arg("port", "9225"));
const CDP_BASE = `http://127.0.0.1:${CDP_PORT}`;
const OUT =
  arg("out", "") ||
  path.join(os.tmpdir(), `codex-ui-theme-audit-${Date.now()}`);
const WORK = fs.mkdtempSync(path.join(os.tmpdir(), "codex-ui-audit-work-"));
fs.mkdirSync(OUT, { recursive: true });
console.log(`[audit] phase=${PHASE} port=${CDP_PORT}\n[audit] OUT=${OUT}\n[audit] WORK=${WORK}`);

const THEMES = ["blue", "dark", "light"];
let child = null;
let cdp = null;

function killApp() {
  killAppTree(child);
}
process.on("exit", killApp);
process.on("SIGINT", () => process.exit(130));
process.on("SIGTERM", () => process.exit(143));

const evalJs = (expression) => cdp.evalJs(expression);
const waitFor = (desc, expr, timeoutMs) => cdp.waitFor(desc, expr, timeoutMs);

async function shot(name) {
  const r = await cdp.send("Page.captureScreenshot", {
    format: "png",
    fromSurface: true,
  });
  const file = path.join(OUT, name);
  fs.writeFileSync(file, Buffer.from(r.data, "base64"));
  const size = fs.statSync(file).size;
  console.log(`[audit] SHOT ${name} (${size} bytes)`);
  return file;
}

async function domState(name) {
  const st = await evalJs(`(() => {
    const q = (s) => document.querySelectorAll(s).length;
    const rects = (sel) => Array.from(document.querySelectorAll(sel)).map((el) => {
      const r = el.getBoundingClientRect();
      const cs = getComputedStyle(el);
      return {
        x: Math.round(r.x), y: Math.round(r.y),
        w: Math.round(r.width), h: Math.round(r.height),
        bg: cs.backgroundColor, color: cs.color,
        cls: el.className,
      };
    });
    const sc = document.querySelector(".chat-scroll");
    return {
      theme: document.documentElement.dataset.theme,
      msgs: q(".msg"),
      userMsgs: q(".msg-user"),
      agentMsgs: q(".msg-agent"),
      errors: q(".msg-error"),
      toolCards: q(".assistant-card"),
      reasoning: q(".msg-reasoning"),
      modal: !!document.querySelector(".modal"),
      toast: document.querySelector(".toast")?.textContent ?? "",
      sendBtn: document.querySelector(".send-btn")?.textContent?.trim() ?? "",
      stopBtn: !!document.querySelector(".send-btn.stop"),
      scrollTop: sc ? Math.round(sc.scrollTop) : null,
      scrollHeight: sc ? sc.scrollHeight : null,
      toolCardsRect: rects(".assistant-card"),
      toolOutputRect: rects(".tool-output"),
      toolCommandRect: rects(".tool-command"),
    };
  })()`);
  fs.writeFileSync(
    path.join(OUT, name.replace(/\.png$/, ".dom.json")),
    JSON.stringify(st, null, 2)
  );
  console.log(`[audit] DOM ${name} -> ${JSON.stringify(st)}`);
  return st;
}

async function setTheme(id) {
  await evalJs(`(() => {
    document.documentElement.dataset.theme = ${JSON.stringify(id)};
    try { localStorage.setItem("codex-ui-theme", ${JSON.stringify(id)}); } catch {}
    return document.documentElement.dataset.theme;
  })()`);
  await sleep(500);
}

async function openSettings() {
  await evalJs(`document.querySelector('button[aria-label="设置"]').click()`);
  await waitFor("设置弹窗", `!!document.querySelector(".settings-modal")`, 5000);
  await sleep(400);
}

async function closeSettings() {
  await evalJs(`document.querySelector(".settings-modal .modal-close").click()`);
  await sleep(300);
}

async function clickThemeCard(id) {
  await evalJs(`document.querySelector('.theme-card[data-theme-id="${id}"]').click()`);
  await sleep(400);
}

async function setInput(text) {
  await evalJs(`(() => {
    const ed = window.__CODEX_UI_EDITOR__;
    if (!ed) throw new Error("编辑器实例未暴露");
    ed.commands.setTextSelection(ed.state.doc.content.size);
    ed.commands.insertContent(${JSON.stringify(text)});
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

async function waitSendEnabled(timeoutMs = 8000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const ok = await evalJs(`(() => {
      const b = document.querySelector("button.send-btn");
      return !!b && !b.disabled;
    })()`);
    if (ok) return;
    await sleep(400);
  }
  throw new Error("发送按钮长时间不可用");
}

// 等待回合结束；期间自动批准可能弹出的提权/操作审批
async function waitTurnDone(desc, timeoutMs = 240000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    await evalJs(`(() => {
      const btn = Array.from(document.querySelectorAll(".modal button")).find(
        (b) => b.textContent.trim() === "批准"
      );
      if (btn) btn.click();
      return true;
    })()`);
    const done = await evalJs(`!document.querySelector(".send-btn.stop")`);
    if (done) return;
    await sleep(800);
  }
  throw new Error(`等待超时: ${desc}`);
}

// 发送后等待按钮进入“停止”态，确认回合真正开始
async function waitTurnStarted(timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs;
  let lastToast = "";
  while (Date.now() < deadline) {
    const started = await evalJs(`!!document.querySelector(".send-btn.stop")`);
    if (started) return;
    const toast = await evalJs(`document.querySelector(".toast")?.textContent ?? ""`);
    if (toast && toast !== lastToast) {
      console.log(`[audit] toast: ${toast}`);
      lastToast = toast;
    }
    await sleep(400);
  }
  const final = await evalJs(`(() => ({
    stop: !!document.querySelector(".send-btn.stop"),
    disabled: document.querySelector("button.send-btn")?.disabled ?? null,
    text: document.querySelector("button.send-btn")?.textContent?.trim() ?? "",
    docLen: window.__CODEX_UI_EDITOR__?.state.doc.textContent.length ?? -1,
    toast: document.querySelector(".toast")?.textContent ?? "",
  }))()`);
  throw new Error(`回合未进入进行中状态: ${JSON.stringify(final)}`);
}

async function scrollToFirstAgent() {
  await evalJs(`(() => {
    const scroller = document.querySelector(".chat-scroll");
    const el = document.querySelector(".msg-agent") || document.querySelector(".msg");
    if (!scroller || !el) return false;
    const top =
      el.getBoundingClientRect().top -
      scroller.getBoundingClientRect().top +
      scroller.scrollTop -
      90;
    scroller.scrollTop = Math.max(0, top);
    return true;
  })()`);
  await sleep(500);
}

async function scrollToBottom() {
  await evalJs(`(() => {
    const scroller = document.querySelector(".chat-scroll");
    if (!scroller) return false;
    scroller.scrollTop = scroller.scrollHeight;
    return true;
  })()`);
  await sleep(500);
}

async function dumpTokens(theme) {
  const tokens = await evalJs(`(() => {
    const cs = getComputedStyle(document.documentElement);
    const out = {};
    for (const v of cs) if (v.startsWith("--")) out[v] = cs.getPropertyValue(v).trim();
    return out;
  })()`);
  fs.writeFileSync(path.join(OUT, `07-${theme}-tokens.json`), JSON.stringify(tokens, null, 2));
  console.log(`[audit] TOKENS 07-${theme}-tokens.json (${Object.keys(tokens).length} vars)`);
}

async function phaseStatic() {
  for (const t of THEMES) {
    await setTheme(t);
    await shot(`01-${t}-empty.png`);
  }
  for (const t of THEMES) {
    await setTheme(t);
    await dumpTokens(t);
    await openSettings();
    await clickThemeCard(t);
    await shot(`04-${t}-settings.png`);
    await closeSettings();
  }
  await setTheme("blue");
  // 历史面板常驻右侧，直接等待挂载后截图
  await waitFor("历史面板", `!!document.querySelector(".history-panel")`, 5000);
  await sleep(600);
  await shot("05-history-shared.png");
}

async function phaseChat() {
  await setTheme("blue");
  await sleep(300);
  await setInput(
    "不要执行任何命令。请用 Markdown 输出：一个二级标题、一个三项项目符号列表、一个 PowerShell 代码块、一个 2 行 3 列的表格。示例内容即可。"
  );
  await waitSendEnabled();
  if (!(await clickSend())) throw new Error("发送失败（按钮不可用）");
  await waitTurnStarted();
  await waitTurnDone("Markdown 回合");
  await sleep(2500);
  for (const t of ["blue", "dark", "light"]) {
    await setTheme(t);
    await scrollToFirstAgent();
    await domState(`02-${t}-conversation.png`);
    await shot(`02-${t}-conversation.png`);
  }

  await setTheme("blue");
  await setInput("请运行命令：echo audit-toolcard-ok；然后报告退出码。");
  await waitSendEnabled();
  if (!(await clickSend())) throw new Error("发送失败（命令回合）");
  await waitTurnStarted();
  await waitTurnDone("命令回合", 300000);
  await sleep(2500);

  await evalJs(`document.querySelector(".assistant-card-toggle")?.click()`);
  await sleep(600);

  for (const t of ["blue", "dark", "light"]) {
    await setTheme(t);
    await scrollToBottom();
    await domState(`03-${t}-toolcard.png`);
    await shot(`03-${t}-toolcard.png`);
  }
}

async function phaseDiff() {
  await setTheme("light");
  await sleep(300);
  await setInput("请创建文件 audit-diff.txt，内容为 hello-diff；创建完成后停止，不要做其他操作。");
  await waitSendEnabled();
  if (!(await clickSend())) throw new Error("发送失败（diff 回合）");
  await waitTurnStarted();
  await waitTurnDone("diff 回合", 300000);
  await sleep(2500);
  await evalJs(`Array.from(document.querySelectorAll(".assistant-card-toggle")).forEach((el) => el.click())`);
  await sleep(800);
  await scrollToBottom();
  const st = await evalJs(`(() => {
    const cs = (sel, prop) => {
      const el = document.querySelector(sel);
      return el ? getComputedStyle(el)[prop] : null;
    };
    return {
      theme: document.documentElement.dataset.theme,
      toolCards: document.querySelectorAll(".assistant-card").length,
      diffViews: document.querySelectorAll(".diff-view").length,
      diffViewBg: cs(".diff-view", "backgroundColor"),
      diffAdd: cs(".diff-add", "color"),
      diffDel: cs(".diff-del", "color"),
      diffFile: cs(".diff-file", "color"),
      diffHunk: cs(".diff-hunk", "color"),
      diffHunkBg: cs(".diff-hunk", "backgroundColor"),
      diffTextColor: cs(".diff-view", "color"),
    };
  })()`);
  fs.writeFileSync(
    path.join(OUT, "06-light-diff.dom.json"),
    JSON.stringify(st, null, 2)
  );
  console.log(`[audit] DIFF-DOM -> ${JSON.stringify(st)}`);
  await shot("06-light-diff.png");
}

async function main() {
  const r = await spawnApp({ cwd: WORK, port: CDP_PORT });
  child = r.child;
  cdp = await createClient(r.page.webSocketDebuggerUrl);
  await waitForEditor(cdp, 60000);
  await sleep(1000);
  console.log(`[audit] APP READY, data-theme=${await evalJs('document.documentElement.dataset.theme')}`);

  if (PHASE === "static") await phaseStatic();
  else if (PHASE === "chat") await phaseChat();
  else if (PHASE === "diff") await phaseDiff();
  else throw new Error(`未知 phase: ${PHASE}`);
  console.log(`[audit] PHASE ${PHASE} DONE`);
}

main()
  .then(() => process.exit(0))
  .catch((e) => {
    console.error("[audit] FAIL", e);
    process.exit(1);
  });
