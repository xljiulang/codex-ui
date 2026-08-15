// codex-ui 目标 flag E2E：勾选后首条消息即目标，终态自动复位
// 流程：无会话勾选/取消勾选 → 勾选后发送首条消息（消息纯文本即目标）自动挂载
//       → 服务端围绕目标自动续跑完成（文件标记）→ 终态自动复位（× 与勾选态消失）
//       → 再次勾选后点停止先清目标
// 用法: node scripts/verify-goal.mjs
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
  sleep,
  spawnApp,
  waitForEditor,
} from "./lib/e2e.mjs";

const CDP_PORT = Number(process.env.CODEX_E2E_PORT || "9223");

const MARKER = "GOMARK_" + crypto.randomBytes(4).toString("hex").toUpperCase();
const MARKER2 = "GOMARK2_" + crypto.randomBytes(4).toString("hex").toUpperCase();
const testDir = mkTmp("codex-ui-goal-test-");
const results = [];
const record = (name, ok, detail = "") =>
  recordResult(results, name, ok, detail);
let child = null;
let cdp = null;

function cleanup() {
  killAppTree(child);
  cleanupSessions([MARKER, MARKER2]);
  try {
    fs.rmSync(testDir, { recursive: true, force: true });
  } catch {}
}

process.on("exit", cleanup);
process.on("SIGINT", () => process.exit(130));
process.on("SIGTERM", () => process.exit(143));

const evalJs = (expression) => cdp.evalJs(expression);
const waitFor = (desc, expr, timeoutMs) =>
  cdp.waitFor(desc, expr, timeoutMs);

/** 在富文本输入框写入文本 */
async function setEditorText(text) {
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

/** 点击目标旗子（toggle 勾选/取消） */
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
}

/** 勾选目标 flag：等 has-goal 出现（待首条消息态，无 ×） */
async function armGoalFlag() {
  await clickGoalFlag();
  await waitFor(
    "目标已勾选（has-goal）",
    `(() => {
      const b = document.querySelector(".goal-icon-btn");
      return !!b && b.classList.contains("has-goal");
    })()`,
    5000,
  );
}

/** 取消勾选（未发送前）：等 has-goal 消失 */
async function disarmGoalFlag() {
  await clickGoalFlag();
  await waitFor(
    "目标已取消勾选",
    `!document.querySelector(".goal-icon-btn.has-goal")`,
    5000,
  );
}

/** 目标激活期间反复点停止，直到目标被清除且空闲；返回期间出现的异常 toast */
async function stopUntilGoalCleared(timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const state = await evalJs(`(() => ({
      active: !!document.querySelector(".send-btn.stop"),
      cleared: !document.querySelector(".goal-icon-btn.has-goal"),
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
  const r = await spawnApp({ cwd: testDir, port: CDP_PORT });
  child = r.child;
  cdp = await createClient(r.page.webSocketDebuggerUrl);
  await waitForEditor(cdp, 60000);

  const goalFile = path.join(testDir, "goal-ok.txt");
  const goalText = `创建文件 ${goalFile}，内容写入 ${MARKER}，完成后停止`;

  // 场景 0：未创建会话时旗子可用；勾选后进入“已勾选待首条消息”态（has-goal、无 ×），可取消勾选
  const flagUsable = await evalJs(`(() => {
    const b = document.querySelector(".goal-icon-btn");
    return !!b && !b.disabled;
  })()`);
  record("目标: 未创建会话时旗子可用", flagUsable === true);

  await armGoalFlag();
  const armedState = await evalJs(`(() => {
    const b = document.querySelector(".goal-icon-btn");
    const badge = document.querySelector(".goal-check");
    return (
      !!b &&
      b.classList.contains("has-goal") &&
      !!badge
    );
  })()`);
  record("目标: 勾选后 has-goal 出现且角标出现（待首条消息）", armedState === true);

  await disarmGoalFlag();
  record(
    "目标: 未发送前可取消勾选（has-goal 消失）",
    (await evalJs(
      `!document.querySelector(".goal-icon-btn.has-goal")`,
    )) === true,
  );

  // 场景 1：勾选后发送首条消息，消息纯文本即目标并自动挂载
  await armGoalFlag();
  await setEditorText(goalText);
  const sent = await clickSend();
  if (!sent) throw new Error("发送失败");
  await waitFor(
    "目标自动挂载（status-active 出现）",
    `!!document.querySelector(".goal-icon-btn.status-active")`,
    60000,
  );
  record("目标: 首条消息即目标并自动挂载", true);

  // 服务端围绕目标自动续跑完成 → 终态自动复位（has-goal 消失、flag 回到未勾选）
  await waitFor(
    "目标完成并自动复位（has-goal 消失）",
    `!document.querySelector(".goal-icon-btn.has-goal")`,
    300000,
  );
  const fileOk =
    fs.existsSync(goalFile) &&
    fs.readFileSync(goalFile, "utf8").includes(MARKER);
  record("目标: 实质性目标已完成（文件含标记词）", fileOk);
  record(
    "目标: 终态自动复位（has-goal 消失）",
    (await evalJs(
      `!document.querySelector(".goal-icon-btn.has-goal")`,
    )) === true,
  );

  // 场景 2：再次勾选后发送新目标消息，目标激活期间点停止 → 先清目标再中断
  await armGoalFlag();
  await setEditorText(
    `分析 ${testDir} 目录下的所有文件并给出架构总结，回复 ${MARKER2}`,
  );
  const sent2 = await clickSend();
  if (!sent2) throw new Error("发送失败");
  await waitFor(
    "目标再次挂载（status-active 出现）",
    `!!document.querySelector(".goal-icon-btn.status-active")`,
    60000,
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
  .then(() => finish(results, cleanup))
  .catch((e) => {
    log("E2E 失败: " + e.message);
    cleanup();
    process.exit(2);
  });
