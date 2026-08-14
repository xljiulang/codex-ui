// 精简 diff 专项 E2E：1 条真实模型消息（apply_patch 改 hello.txt + 新建 sample.ts）
// 验证独立 diff 窗口：内联旧/新/分隔、指示条对齐、代码语法高亮、自定义右键菜单。
// 用法: node scripts/verify-diff.mjs
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import {
  cleanupSessions,
  createClient,
  finish,
  killAppTree,
  log,
  mkTmp,
  record as recordResult,
  spawnApp,
  waitForEditor,
} from "./lib/e2e.mjs";

const CDP_PORT = Number(process.env.CODEX_E2E_PORT || "9222");
const CDP_BASE = `http://127.0.0.1:${CDP_PORT}`;
const TAG = "[测试diff]";
const MARKER = "DIFF_MARKER_" + crypto.randomBytes(4).toString("hex").toUpperCase();

const testDir = mkTmp("codexui-diff-e2e-");
// 300 行大文件：让 diff 窗口出现滚动，验证指示条与滚动条位置对齐
const helloLines = Array.from({ length: 300 }, (_, i) =>
  i === 99 ? `标记词: ${MARKER}` : `第 ${i + 1} 行普通内容`,
);
fs.writeFileSync(path.join(testDir, "hello.txt"), helloLines.join("\n") + "\n", "utf8");
const evidenceDir = mkTmp("codexui-diff-evidence-");

let child = null;
let cdp = null;
const results = [];
const record = (name, ok, detail = "") =>
  recordResult(results, name, ok, detail);

function cleanup() {
  killAppTree(child);
  cleanupSessions([MARKER]);
  try {
    fs.rmSync(testDir, { recursive: true, force: true });
  } catch {}
}

process.on("exit", cleanup);
process.on("SIGINT", () => process.exit(130));
process.on("SIGTERM", () => process.exit(143));

async function main() {
  const r = await spawnApp({ cwd: testDir, port: CDP_PORT });
  child = r.child;
  const mainPageId = r.page.id;
  cdp = await createClient(r.page.webSocketDebuggerUrl);
  await waitForEditor(cdp, 60000);

  const setInput = (text) =>
    cdp.evalJs(`(() => {
      const ed = window.__CODEX_UI_EDITOR__;
      if (!ed) throw new Error("编辑器实例未暴露");
      ed.commands.setTextSelection(ed.state.doc.content.size);
      ed.commands.insertContent(${JSON.stringify(text)});
      return ed.getText();
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

  // ToolCard 默认折叠：先展开“文件变更”卡片，变更行才会出现在 DOM
  await cdp.evalJs(`(() => {
    const cards = Array.from(document.querySelectorAll(".tool-card"));
    const c = cards.find((x) => x.querySelector(".tool-card-title")?.textContent.trim() === "文件变更");
    const header = c?.querySelector(".tool-card-header");
    if (header) header.click();
  })()`);
  await sleep(500);

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
  .then(() => finish(results, cleanup))
  .catch((e) => { log("E2E 失败: " + e.message); cleanup(); process.exit(2); });
