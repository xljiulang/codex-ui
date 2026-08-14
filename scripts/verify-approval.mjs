// 端到端验证：请求批准模式下批准文件变更，确认不再显示"已拒绝"，并检查 diff 渲染
function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
const CDP_PORT = Number(arg("port", process.env.CODEX_E2E_PORT || "9222"));
const CDP_BASE = `http://127.0.0.1:${CDP_PORT}`;
const TARGET = process.env.CODEX_E2E_TARGET;
if (!TARGET) {
  console.error("需要 CODEX_E2E_TARGET");
  process.exit(1);
}
const PROMPT = `编辑文件：在 ${TARGET} 中写入两行文字 hello 和 world`;

const targets = await (await fetch(`${CDP_BASE}/json/list`)).json();
const page = targets.find((t) => t.type === "page");
if (!page) {
  console.error(
    `NO_PAGE：未检测到运行中的 codex-ui（CDP ${CDP_PORT}）。` +
      `本探针附着运行中的应用，请先以远程调试端口启动，例如：\n` +
      `  set WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=${CDP_PORT} && codex-ui.exe\n` +
      `或改用 scripts/run-e2e.mjs 自动编排（--build 后串行运行）。`,
  );
  process.exit(1);
}
const ws = new WebSocket(page.webSocketDebuggerUrl);
const timer = setTimeout(() => {
  console.log("CDP_TIMEOUT");
  process.exit(1);
}, 150000);
let id = 0;
function evalJs(expression, awaitPromise = true) {
  return new Promise((resolve, reject) => {
    const mid = ++id;
    const handler = (e) => {
      const m = JSON.parse(e.data);
      if (m.id === mid) {
        ws.removeEventListener("message", handler);
        if (m.result?.exceptionDetails) {
          reject(new Error(JSON.stringify(m.result.exceptionDetails)));
        } else {
          resolve(m.result?.result?.value);
        }
      }
    };
    ws.addEventListener("message", handler);
    ws.send(
      JSON.stringify({
        id: mid,
        method: "Runtime.evaluate",
        params: { expression, awaitPromise, returnByValue: true },
      }),
    );
  });
}
const sleep = (ms) => evalJs(`new Promise(r => setTimeout(r, ${ms}))`);

ws.onopen = async () => {
  try {
    // 先新建会话，避免沿用已被删除的会话
    await evalJs(
      `document.querySelector('button[aria-label="新建会话"]').click()`,
    );
    await sleep(400);
    await evalJs(`(() => {
      const ed = window.__CODEX_UI_EDITOR__;
      if (!ed) throw new Error("编辑器实例未暴露");
      ed.commands.setTextSelection(ed.state.doc.content.size);
      ed.commands.insertContent(${JSON.stringify(PROMPT)});
    })()`);
    for (let i = 0; i < 20; i++) {
      await sleep(200);
      const st = await evalJs(
        `(() => { const b = document.querySelector('.composer-right .send-btn'); const ed = window.__CODEX_UI_EDITOR__; return JSON.stringify({ val: (ed ? ed.getText() : "").slice(0, 8), disabled: b ? b.disabled : 'no-btn' }); })()`,
      );
      const s = JSON.parse(st);
      if (s.val && s.disabled === false) break;
    }
    await evalJs(`document.querySelector('.composer-right .send-btn').click()`);

    let clicks = 0;
    let sawStop = false;
    let finished = false;
    const clickedLabels = [];
    for (let i = 0; i < 220; i++) {
      await sleep(300);
      const st = await evalJs(`(() => {
        const modal = document.querySelector('.modal');
        if (modal) {
          const ruleBtn = [...modal.querySelectorAll('.btn')].find(b => b.textContent.includes('记住此规则'));
          const primary = modal.querySelector('.btn.primary');
          const btn = ruleBtn || primary;
          if (!btn) return 'no-btn';
          const label = btn.textContent.trim();
          btn.click();
          return 'clicked:' + label;
        }
        const stop = !!document.querySelector('.send-btn.stop');
        const fc = [...document.querySelectorAll('.tool-card')]
          .find(c => c.querySelector('.tool-card-title')?.textContent === '文件变更');
        return JSON.stringify({ modal: false, stop, fcStatus: fc ? fc.querySelector('.tool-card-status')?.textContent.trim() : null });
      })()`);
      if (typeof st === "string" && st.startsWith("clicked:")) {
        clickedLabels.push(st);
        clicks++;
        continue;
      }
      if (st === "no-btn" || st === "no-modal") {
        continue;
      }
      const s = JSON.parse(st);
      if (s.stop) sawStop = true;
      if (sawStop && !s.stop) {
        finished = true;
        break;
      }
      if (i === 219) {
        console.log("LAST_STATE:", st);
      }
    }
    console.log("FINISHED:", finished, "CLICKS:", clickedLabels);
    await sleep(1000);
    const card = await evalJs(`(() => {
      const cards = [...document.querySelectorAll('.tool-card')];
      const all = cards.map(c => c.querySelector('.tool-card-title')?.textContent + ':' + (c.querySelector('.tool-card-status')?.textContent.trim() || ''));
      const fc = cards.find(c => c.querySelector('.tool-card-title')?.textContent === '文件变更');
      if (!fc) return JSON.stringify({ fc: false });
      const status = fc.querySelector('.tool-card-status')?.textContent.trim() || '';
      const adds = [...fc.querySelectorAll('.diff-add')].map(d => d.textContent);
      const dels = [...fc.querySelectorAll('.diff-del')].map(d => d.textContent);
      return JSON.stringify({ fc: true, status, adds, dels, all });
    })()`);
    console.log("FILECHANGE_CARD:", card);

    try {
      const cleanup = await evalJs(`(async () => {
        const r = await window.__TAURI_INTERNALS__.invoke('thread_list', { limit: 50, cursor: null, cwd: null });
        const ts = r.data.filter(x => (x.preview || '').startsWith('编辑文件'));
        for (const t of ts) { try { await window.__TAURI_INTERNALS__.invoke('thread_delete', { threadId: t.id }); } catch {} }
        return 'deleted ' + ts.length;
      })()`);
      console.log("CLEANUP:", cleanup);
    } catch (e) {
      console.log("CLEANUP_ERR:", String(e));
    }
    clearTimeout(timer);
    process.exit(0);
  } catch (e) {
    console.log("ERR:", String(e));
    clearTimeout(timer);
    process.exit(1);
  }
};
