// codex-ui 核心链路 E2E：置顶（Pinned 分区协议）、切换会话自动停止旧回合、
// 设置面板 toggle、Ctrl+Enter 换行、置顶徽章对齐（DOM 几何断言 + 截图）。
// 通过 WebView2 远程调试（CDP）驱动真实 release UI + 少量真实模型调用。
// 用法: node scripts/verify-core.mjs
import crypto from "node:crypto";
import fs from "node:fs";
import {
  cleanupSessions,
  createClient,
  ensureSession,
  finish,
  killAppTree,
  log,
  mkTmp,
  record as recordResult,
  sleep,
  spawnApp,
} from "./lib/e2e.mjs";

const CDP_PORT = Number(process.env.CODEX_E2E_PORT || "9222");

const TAG_A = "[测试核心A]";
const TAG_B = "[测试核心B]";
const MARKER = "CORE_" + crypto.randomBytes(4).toString("hex").toUpperCase();
// 提示需 ≤15 字：避免触发“标题自动总结”改写会话名，保证历史行仍含 TAG 文本
const PROMPT_A = `${TAG_A} 只回 OK`;
const PROMPT_B = `${TAG_B} 只回 OK`;
const LONG_PROMPT = `[测试核心L] ${MARKER} 请每秒输出一行数字，循环 12 次后输出 DONE 结束。`;

const testDir = mkTmp("codex-ui-core-test-");
const evidenceDir = mkTmp("codex-ui-core-evidence-");

const results = [];
const record = (name, ok, detail = "") =>
  recordResult(results, name, ok, detail);
let child = null;
let cdp = null;

function cleanup() {
  killAppTree(child);
  cleanupSessions([MARKER, TAG_A, TAG_B, "[测试核心L]"]);
  try {
    fs.rmSync(testDir, { recursive: true, force: true });
  } catch {}
}

process.on("exit", cleanup);
process.on("SIGINT", () => process.exit(130));
process.on("SIGTERM", () => process.exit(143));

const evalJs = (expression) => cdp.evalJs(expression);
const screenshot = (name) => cdp.screenshot(name, evidenceDir);
const waitFor = (desc, expr, timeoutMs) =>
  cdp.waitFor(desc, expr, timeoutMs);

async function launchApp() {
  const r = await spawnApp({ cwd: testDir, port: CDP_PORT });
  child = r.child;
  cdp = await createClient(r.page.webSocketDebuggerUrl);
  await ensureSession(cdp);
  log("UI 就绪");
}

async function relaunchApp() {
  log("重启应用以验证置顶持久化…");
  killAppTree(child);
  child = null;
  cdp = null;
  await sleep(2000);
  await launchApp();
}

async function setInput(text) {
  await evalJs(`(() => {
    const ed = window.__CODEX_UI_EDITOR__;
    if (!ed) throw new Error("编辑器实例未暴露");
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

async function clickSend() {
  // 插入内容后按钮禁用态随 Vue 渲染更新，轮询等待（≤5s）再点击
  const deadline = Date.now() + 5000;
  let last;
  while (Date.now() < deadline) {
    last = await evalJs(`(() => {
      // 多会话标签下 v-show 隐藏的旧会话仍在 DOM：只认可见（活动）会话的按钮
      const b = Array.from(document.querySelectorAll("button.send-btn")).find(
        (x) => x.offsetParent !== null,
      );
      if (!b) return { ok: false, reason: "send-btn 不存在" };
      if (b.disabled) return { ok: false, reason: "send-btn 禁用" };
      b.click();
      return { ok: true };
    })()`);
    if (last.ok) return last;
    await sleep(200);
  }
  const diag = await evalJs(`(() => {
    const ed = window.__CODEX_UI_EDITOR__;
    return JSON.stringify({
      text: ed ? ed.getText() : null,
      docSize: ed ? ed.state.doc.content.size : -1,
      attachmentChips: document.querySelectorAll(".attachment-chip").length,
      sessionTabs: Array.from(document.querySelectorAll(".session-tab")).map((t) => t.textContent.trim()),
      activeTab: document.querySelector(".session-tab.active")?.textContent.trim() ?? null,
      hasStop: !!document.querySelector(".send-btn.stop"),
    });
  })()`);
  return { ok: false, reason: `send-btn 禁用 (${diag})` };
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

async function sendPrompt(text) {
  await setInput(text);
  const r = await clickSend();
  if (!r.ok) throw new Error("发送失败: " + r.reason);
}

async function openHistory() {
  // 面板常驻右侧：只需等待挂载
  await waitFor(
    "历史面板出现",
    `!!document.querySelector(".history-view")`,
    10000,
  );
  await expandAllFolders();
  const hasRows = await evalJs(
    `document.querySelectorAll(".history-item, .history-folder").length > 0`,
  );
  if (!hasRows) {
    // 面板无法靠开关重挂载；重启应用触发启动时全量拉取
    await relaunchApp();
    await expandAllFolders();
  }
}

/** 历史目录默认收起；轮询等待列表渲染完成，并把全部折叠目录展开 */
async function expandAllFolders() {
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    const state = await evalJs(`(() => {
      const collapsed = document.querySelectorAll(
        '.history-folder[aria-expanded="false"]',
      );
      const total = document.querySelectorAll(".history-folder").length;
      return { collapsed: collapsed.length, total };
    })()`);
    if (state.total > 0 && state.collapsed === 0) return;
    if (state.collapsed > 0) {
      await evalJs(`(() => {
        const fs = Array.from(
          document.querySelectorAll('.history-folder[aria-expanded="false"]'),
        );
        for (const f of fs) f.click();
        return fs.length;
      })()`);
    }
    await sleep(500);
  }
}

async function clickNewChat() {
  await evalJs(`(async () => {
    const t = window.__CODEX_UI_TEST__;
    if (!t) throw new Error("__CODEX_UI_TEST__ 未暴露");
    await t.newSession();
  })()`);
}

async function historyRowCount() {
  return evalJs(
    `document.querySelectorAll(".history-item").length`,
  );
}

async function clickPinInRow(text) {
  return evalJs(`(() => {
    const rows = Array.from(document.querySelectorAll(".history-item"));
    const r = rows.find((x) => x.innerText.includes(${JSON.stringify(text)}));
    if (!r) return false;
    r.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 300,
      clientY: 300,
    }));
    const menu = document.querySelector(".ctx-menu");
    const b = menu
      ? Array.from(menu.querySelectorAll(".ctx-menu-item")).find(
          (x) => x.innerText.trim() === "置顶" || x.innerText.trim() === "取消置顶",
        )
      : null;
    if (!b) return false;
    b.click();
    return true;
  })()`);
}

async function pinBadgeEdges() {
  return evalJs(`(() => {
    const groups = { folder: [], flat: [] };
    for (const b of Array.from(document.querySelectorAll(".history-item .pin-badge"))) {
      const key = b.closest(".folder-item") ? "folder" : "flat";
      groups[key].push(Math.round(b.getBoundingClientRect().left));
    }
    return groups;
  })()`);
}

async function firstRowsText() {
  return evalJs(
    `Array.from(document.querySelectorAll(".history-item")).slice(0, 3).map((r) => r.innerText.slice(0, 40))`,
  );
}

// ---------- 场景 1: 设置面板 toggle ----------
async function scenarioSettingsToggle() {
  log("场景 1: 设置面板 toggle（历史面板常驻右侧）");
  await evalJs(`document.querySelector('button[aria-label="设置"]').click()`);
  await waitFor("设置面板出现", `!!document.querySelector(".settings")`, 10000);
  const opened = await evalJs(
    `!!document.querySelector(".settings") && !!document.querySelector(".history-view")`,
  );
  record("设置: 点击打开设置面板，历史面板保持显示", opened);

  await evalJs(`document.querySelector('button[aria-label="设置"]').click()`);
  await waitFor("设置面板关闭", `!document.querySelector(".settings")`, 10000);
  const historyKept = await evalJs(
    `!!document.querySelector(".history-view")`,
  );
  record("设置: 再次点击关闭设置面板，历史面板仍显示", historyKept);
}

// ---------- 场景 2: Ctrl+Enter 换行 ----------
async function scenarioCtrlEnter() {
  log("场景 2: Ctrl+Enter 换行（enter_to_send 开启）");
  const before = await evalJs(
    `document.querySelectorAll(".msg-user").length`,
  );
  await setInput("第一行文本");
  await evalJs(`(() => {
    const el = document.querySelector(".ProseMirror");
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true }));
  })()`);
  await sleep(500);
  const after = await evalJs(
    `document.querySelectorAll(".msg-user").length`,
  );
  const editorState = await evalJs(`(() => {
    const el = document.querySelector(".ProseMirror");
    const ed = window.__CODEX_UI_EDITOR__;
    return { html: el?.innerHTML ?? "", text: ed ? ed.getText() : "" };
  })()`);
  const inserted =
    editorState.html.includes("<br") || editorState.text.includes("\n");
  record(
    "Ctrl+Enter: 插入硬换行且不发送消息",
    before === after && inserted,
    `before=${before} after=${after} html=${editorState.html.slice(0, 80)}`,
  );
  await clearInput();
}

// ---------- 场景 2b: 粘贴图片添加附件 ----------
async function scenarioPasteImage() {
  log("场景 2b: 粘贴图片添加附件");
  await evalJs(`(() => {
    const bytes = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);
    const file = new File([bytes], "e2e-clip.png", { type: "image/png" });
    const dt = {
      items: [{ kind: "file", type: "image/png", getAsFile: () => file }],
      files: [file],
      types: ["Files"],
      getData: () => "",
    };
    const ev = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(ev, "clipboardData", { value: dt });
    document.querySelector(".ProseMirror").dispatchEvent(ev);
  })()`);
  await waitFor(
    "粘贴后图片缩略图出现",
    `!!document.querySelector(".attachment-thumb")`,
    15000,
  );
  const thumbSrc = await evalJs(
    `document.querySelector(".attachment-thumb")?.getAttribute("src") ?? ""`,
  );
  record(
    "粘贴: 图片进入附件区并渲染缩略图（落盘成功）",
    thumbSrc.includes("asset.localhost"),
    thumbSrc.slice(0, 120),
  );
  const hasThumb = await evalJs(
    `!!document.querySelector(".attachment-chip .attachment-thumb")`,
  );
  record("粘贴: 附件区存在图片缩略图 chip", hasThumb);
  await screenshot("2b-paste-image.png");
  await clearInput();
}

// ---------- 场景 2c: 输入框高度拖拽 ----------
async function scenarioEditorResize() {
  log("场景 2c: 输入框高度拖拽调节");
  await evalJs(`(() => {
    const handle = document.querySelector(".composer-resize-handle");
    handle.dispatchEvent(new PointerEvent("pointerdown", { clientY: 300, bubbles: true }));
  })()`);
  await evalJs(`(() => {
    window.dispatchEvent(new PointerEvent("pointermove", { clientY: 100 }));
    window.dispatchEvent(new PointerEvent("pointerup", {}));
  })()`);
  await sleep(300);
  const style1 = await evalJs(
    `document.querySelector(".composer-input-row").getAttribute("style") ?? ""`,
  );
  const h1 = await evalJs(
    `Math.round(document.querySelector(".ProseMirror").getBoundingClientRect().height)`,
  );
  record(
    "输入框: 向上拖拽后设置内联高度并增高",
    style1.includes("--composer-h") && h1 >= 120,
    `style=${style1} h=${h1}`,
  );
  // 拖回最低高度
  await evalJs(`(() => {
    const handle = document.querySelector(".composer-resize-handle");
    handle.dispatchEvent(new PointerEvent("pointerdown", { clientY: 100, bubbles: true }));
  })()`);
  await evalJs(`(() => {
    window.dispatchEvent(new PointerEvent("pointermove", { clientY: 10000 }));
    window.dispatchEvent(new PointerEvent("pointerup", {}));
  })()`);
  await sleep(300);
  const style2 = await evalJs(
    `document.querySelector(".composer-input-row").getAttribute("style") ?? ""`,
  );
  record(
    "输入框: 向下拖拽夹紧到最低高度 120px",
    /--composer-h:\s*120px/.test(style2),
    style2,
  );
}

// ---------- 场景 3: 置顶（Pinned 分区）+ 徽章对齐 + 持久化 ----------
async function scenarioPin() {
  log("场景 3: 置顶 / 取消置顶 / 徽章对齐 / 持久化");
  await sendPrompt(PROMPT_A);
  await waitTurnDone();
  await clickNewChat();
  await waitFor("新会话就绪", `!!document.querySelector(".ProseMirror")`, 15000);
  await sendPrompt(PROMPT_B);
  await waitTurnDone();

  await openHistory();
  await waitFor(
    "历史行出现",
    `Array.from(document.querySelectorAll(".history-item")).some((r) => r.innerText.includes(${JSON.stringify(
      TAG_A,
    )}))`,
    30000,
  );
  const rows = await historyRowCount();
  record("置顶: 历史面板列出测试会话", rows >= 2, `rows=${rows}`);

  await clickPinInRow(TAG_A);
  await waitFor(
    "徽章 A 出现",
    `(() => { const r = Array.from(document.querySelectorAll(".history-item")).find((x) => x.innerText.includes(${JSON.stringify(
      TAG_A,
    )})); return !!r?.querySelector(".pin-badge"); })()`,
    15000,
  );
  await clickPinInRow(TAG_B);
  await waitFor(
    "徽章 B 出现",
    `(() => { const r = Array.from(document.querySelectorAll(".history-item")).find((x) => x.innerText.includes(${JSON.stringify(
      TAG_B,
    )})); return !!r?.querySelector(".pin-badge"); })()`,
    15000,
  );

  const top = await firstRowsText();
  const pinLayout = await evalJs(`(() => {
    const rows = Array.from(document.querySelectorAll(".history-item"));
    const badges = rows.map((r) => !!r.querySelector(".pin-badge"));
    const idxA = rows.findIndex((r) => r.innerText.includes(${JSON.stringify(
      TAG_A,
    )}));
    const idxB = rows.findIndex((r) => r.innerText.includes(${JSON.stringify(
      TAG_B,
    )}));
    return {
      idxA,
      idxB,
      badgeA: idxA >= 0 ? badges[idxA] : false,
      badgeB: idxB >= 0 ? badges[idxB] : false,
      rows: rows.length,
    };
  })()`);
  record(
    "置顶: 测试会话已置顶且同目录内相邻",
    pinLayout.idxA >= 0 &&
      pinLayout.idxB >= 0 &&
      Math.abs(pinLayout.idxA - pinLayout.idxB) === 1 &&
      pinLayout.badgeA &&
      pinLayout.badgeB,
    `${JSON.stringify(pinLayout)} top=${JSON.stringify(top)}`,
  );
  const edges = await pinBadgeEdges();
  const aligned = Object.values(edges).every(
    (arr) => arr.length < 2 || Math.max(...arr) - Math.min(...arr) <= 1,
  );
  record(
    "置顶: 置顶图标垂直对齐（同缩进内 ≤1px）",
    aligned,
    `edges=${JSON.stringify(edges)}`,
  );
  await screenshot("3-pinned.png");

  await relaunchApp();
  await openHistory();
  let badgeCount = 0;
  const badgeDeadline = Date.now() + 30000;
  while (Date.now() < badgeDeadline) {
    badgeCount = await evalJs(
      `document.querySelectorAll(".history-item .pin-badge").length`,
    );
    if (badgeCount >= 2) break;
    await sleep(1000);
  }
  if (badgeCount < 2) {
    const dump = await evalJs(`(() => ({
      panel: !!document.querySelector(".history-view"),
      items: document.querySelectorAll(".history-item").length,
      folders: document.querySelectorAll(".history-folder").length,
      rows: Array.from(document.querySelectorAll(".history-item")).slice(0, 8).map((r) => ({
        t: r.querySelector(".history-title")?.innerText ?? "",
        p: !!r.querySelector(".pin-badge"),
      })),
      body: document.body.innerText.slice(0, 300),
    }))()`);
    log("重启后徽章不足，当前历史列表: " + JSON.stringify(dump));
  }
  record("置顶: 重启后徽章出现（≥2）", badgeCount >= 2, `badges=${badgeCount}`);
  const persisted = await evalJs(
    `(() => { const rows = Array.from(document.querySelectorAll(".history-item")); const a = rows.find((x) => x.innerText.includes(${JSON.stringify(
      TAG_A,
    )})); const b = rows.find((x) => x.innerText.includes(${JSON.stringify(
      TAG_B,
    )})); return !!(a?.querySelector(".pin-badge") && b?.querySelector(".pin-badge")); })()`,
  );
  record("置顶: 重启后置顶状态持久（Pinned 分区服务端存储）", persisted);
  await screenshot("3-persisted.png");

  await clickPinInRow(TAG_A);
  await clickPinInRow(TAG_B);
  await waitFor(
    "取消置顶后徽章消失",
    `(() => { const rows = Array.from(document.querySelectorAll(".history-item")); const a = rows.find((x) => x.innerText.includes(${JSON.stringify(
      TAG_A,
    )})); const b = rows.find((x) => x.innerText.includes(${JSON.stringify(
      TAG_B,
    )})); return !a?.querySelector(".pin-badge") && !b?.querySelector(".pin-badge"); })()`,
    15000,
  );
  const unpinned = await evalJs(
    `(() => { const rows = Array.from(document.querySelectorAll(".history-item")); const a = rows.find((x) => x.innerText.includes(${JSON.stringify(
      TAG_A,
    )})); const b = rows.find((x) => x.innerText.includes(${JSON.stringify(
      TAG_B,
    )})); return !a?.querySelector(".pin-badge") && !b?.querySelector(".pin-badge"); })()`,
  );
  record("置顶: 取消置顶后徽章消失", unpinned);
}

// ---------- 场景 4: 切换会话自动停止旧回合 ----------
async function scenarioStopOnSwitch() {
  log("场景 4: 切换会话自动停止旧回合");
  const opened = await evalJs(`(() => {
    const rows = Array.from(document.querySelectorAll(".history-item"));
    const r = rows.find((x) => x.innerText.includes(${JSON.stringify(TAG_A)}));
    if (!r) return false;
    r.click();
    return true;
  })()`);
  if (!opened) throw new Error("未找到会话 A 行");
  await waitFor(
    "会话 A 打开",
    `!!document.querySelector(".ProseMirror")`,
    30000,
  );

  await sendPrompt(LONG_PROMPT);
  await waitFor(
    "回合进入进行中（停止按钮出现）",
    `!!document.querySelector(".send-btn.stop")`,
    60000,
  );
  await clickNewChat();
  // 会话进行中切换需先确认（今晚新增的会话切换确认）
  await waitFor(
    "切换确认框出现",
    `!!document.querySelector(".modal .btn.danger")`,
    10000,
  );
  await evalJs(`document.querySelector(".modal .btn.danger").click()`);
  await waitFor(
    "切换确认框关闭",
    `!document.querySelector(".modal")`,
    10000,
  );
  await waitFor(
    "切换后旧回合被自动停止（停止按钮消失）",
    `!document.querySelector(".send-btn.stop")`,
    60000,
  );
  await sleep(3000);
  const stillIdle = await evalJs(
    `!document.querySelector(".send-btn.stop")`,
  );
  const newChatEmpty = await evalJs(
    `document.querySelectorAll(".msg-user").length === 0`,
  );
  record(
    "切换停止: 新建会话后旧回合自动中断，无残留进行中状态",
    stillIdle && newChatEmpty,
  );
}

// ---------- 场景 5: 历史面板常驻 ----------
async function scenarioHistoryAlwaysVisible() {
  log("场景 5: 历史面板常驻右侧（点击会话不关闭）");
  await openHistory();
  await waitFor(
    "历史行出现",
    `document.querySelectorAll(".history-item").length > 0`,
    15000,
  );
  const clicked = await evalJs(`(() => {
    const rows = Array.from(document.querySelectorAll(".history-item"));
    const r = rows.find((x) => x.innerText.includes(${JSON.stringify(TAG_A)}));
    if (!r) return false;
    r.click();
    return true;
  })()`);
  await sleep(1200);
  const stillOpen = await evalJs(
    `!!document.querySelector(".history-view")`,
  );
  record(
    "历史: 点击历史会话后面板保持显示（面板常驻右侧）",
    clicked && stillOpen,
  );
}

async function main() {
  await launchApp();

  await scenarioSettingsToggle();
  await scenarioCtrlEnter();
  await scenarioPasteImage();
  await scenarioEditorResize();
  await scenarioPin();
  await scenarioStopOnSwitch();
  await scenarioHistoryAlwaysVisible();

  const pass = results.filter((r) => r.ok).length;
  log(`\n===== 结果汇总: ${pass}/${results.length} 通过 =====`);
  for (const r of results) {
    log(`${r.ok ? "PASS" : "FAIL"} ${r.name}`);
  }
  log(`证据目录: ${evidenceDir}`);
}

main()
  .then(() => finish(results, cleanup))
  .catch((e) => {
    log("E2E 失败: " + e.message);
    cleanup();
    process.exit(2);
  });
