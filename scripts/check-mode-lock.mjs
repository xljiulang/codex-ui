// 端到端验证：对话进行中权限/任务模式按钮禁用，空闲时恢复可用
const targets = await (await fetch("http://127.0.0.1:9222/json/list")).json();
const page = targets.find((t) => t.type === "page");
if (!page) {
  console.log("NO_PAGE");
  process.exit(1);
}
const ws = new WebSocket(page.webSocketDebuggerUrl);
const timer = setTimeout(() => {
  console.log("CDP_TIMEOUT");
  process.exit(1);
}, 120000);
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

function chipState() {
  return evalJs(`(() => {
    const p = document.querySelector('.perm-chip');
    const t = document.querySelector('.task-chip');
    return JSON.stringify({
      permDisabled: p ? p.disabled : 'no-chip',
      taskDisabled: t ? t.disabled : 'no-chip',
      stop: !!document.querySelector('.send-btn.stop'),
      toast: document.querySelector('.toast')?.textContent?.trim() ?? null,
    });
  })()`);
}

ws.onopen = async () => {
  try {
    // 等待 UI 就绪
    for (let i = 0; i < 30; i++) {
      await sleep(300);
      const ready = await evalJs(`!!document.querySelector('.perm-chip') && !!document.querySelector('.task-chip')`);
      if (ready) break;
    }
    let st = JSON.parse(await chipState());
    console.log("INITIAL:", JSON.stringify(st));

    // 若已有回合在跑，先停止并等待结束
    if (st.stop) {
      await evalJs(`document.querySelector('.send-btn.stop')?.click()`);
      for (let i = 0; i < 60; i++) {
        await sleep(300);
        st = JSON.parse(await chipState());
        if (!st.stop) break;
      }
      console.log("AFTER_STOP:", JSON.stringify(st));
    }

    // 新建对话，避免受历史会话影响
    await evalJs(`document.querySelector('.icon-btn[title="新建对话"]')?.click()`);
    await sleep(400);
    st = JSON.parse(await chipState());
    console.log("NEW_CHAT:", JSON.stringify(st));
    if (st.permDisabled !== false || st.taskDisabled !== false) {
      throw new Error("空闲时模式按钮应为可用");
    }

    // 发送一条简单消息进入回合
    await evalJs(`(() => {
      const ta = document.querySelector('.composer textarea');
      const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
      setter.call(ta, '回复一个词：好的');
      ta.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    await evalJs(`document.querySelector('.composer-right .send-btn').click()`);

    let sawDisabled = false;
    for (let i = 0; i < 90; i++) {
      await sleep(300);
      st = JSON.parse(await chipState());
      if (st.stop && st.permDisabled === true && st.taskDisabled === true) {
        sawDisabled = true;
        break;
      }
      if (st.toast) console.log("TOAST_DETECTED:", st.toast);
    }
    console.log("DURING_TURN:", JSON.stringify(st));
    if (!sawDisabled) throw new Error("回合进行中模式按钮未禁用");
    if (st.toast) throw new Error("回合进行中不应再出现提示 toast");

    // 等待回合完成，按钮应恢复可用
    let done = false;
    for (let i = 0; i < 120; i++) {
      await sleep(300);
      st = JSON.parse(await chipState());
      if (!st.stop && st.permDisabled === false && st.taskDisabled === false) {
        done = true;
        break;
      }
    }
    console.log("AFTER_TURN:", JSON.stringify(st));
    if (!done) throw new Error("回合结束后模式按钮未恢复可用");

    // 清理测试会话
    try {
      const cleanup = await evalJs(`(async () => {
        const r = await window.__TAURI_INTERNALS__.invoke('thread_list', { limit: 50, cursor: null, cwd: null });
        const ts = r.data.filter(x => (x.preview || '').startsWith('回复一个词'));
        for (const t of ts) { try { await window.__TAURI_INTERNALS__.invoke('thread_delete', { threadId: t.id }); } catch {} }
        return 'deleted ' + ts.length;
      })()`);
      console.log("CLEANUP:", cleanup);
    } catch (e) {
      console.log("CLEANUP_ERR:", String(e));
    }
    console.log("RESULT: PASS");
    clearTimeout(timer);
    process.exit(0);
  } catch (e) {
    console.log("ERR:", String(e));
    clearTimeout(timer);
    process.exit(1);
  }
};
