// codex-ui 微信接入 sidecar：包装 wechat-channel 库，经 stdio 换行分隔 JSON
// 与 Rust 微信桥通信。协议细节全部由库承担，本文件只做命令转发与事件回传。
//
// Rust -> sidecar（每行一个对象）：
//   {cmd:"init", stateDir}
//   {cmd:"login", timeoutMs?}
//   {cmd:"start", accountId}
//   {cmd:"stop"}
//   {cmd:"send_text", accountId, toUserId, text}
//   {cmd:"logout", accountId}
// sidecar -> Rust：
//   {event:"ready"}
//   {event:"qr", content}
//   {event:"login_result", success, message, accountId?, userId?}
//   {event:"accounts", accounts:[{id,name?,configured,userId?}]}
//   {event:"session_status", status:{accountId,status,...}}
//   {event:"message", message:{id,accountId,from,to,timestamp,contextToken,text?}}
//   {event:"error", message, kind?, accountId?}

import readline from "node:readline";
import { WeixinClient } from "wechat-channel";

/** 向 Rust 输出一行事件（stdout 专用；日志类输出全部走 stderr 避免污染通道）。 */
function emit(obj) {
  try {
    process.stdout.write(JSON.stringify(obj) + "\n");
  } catch {
    // stdout 断开（宿主退出）时静默退出
    process.exit(0);
  }
}

let client = null;
let clientReady = false;
/** 当前在途二维码登录：{ aborter, loginId }；新 login 会 abort 并接管，login_cancel 取消。 */
let currentLogin = null;
/** 兜底登录序号（桥端未带 loginId 时用）。 */
let loginIdSeq = 0;
/** 首次 init 的在途 Promise：并发/重复 init 幂等。 */
let initPromise = null;
/** 最近一次 init 的 stateDir：login 若早于 init 到达时用于补齐初始化。 */
let lastStateDir = null;

/** 发送串行化：保证回复顺序与入队顺序一致，避免长轮询并发写冲突。 */
let sendChain = Promise.resolve();
function enqueueSend(fn) {
  const run = sendChain.then(fn);
  // 吞掉链上前序失败，保持链条可用；错误由 fn 内部上报为 error 事件。
  sendChain = run.catch(() => {});
  return run;
}

/** 库的 log 钩子：登录期打印「请扫描二维码: <url>」，从中提取二维码内容。 */
function makeLogHook() {
  let captureNextQr = false;
  return (msg) => {
    const text = String(msg ?? "");
    if (/请扫描二维码/.test(text)) captureNextQr = true;
    const m = text.match(/https?:\/\/\S+/)?.[0] ?? text.match(/\bwxp:\/\/\S+/)?.[0];
    if (m && (captureNextQr || /二维码|qrcode/i.test(text))) {
      captureNextQr = false;
      emit({ event: "qr", content: m });
      return;
    }
    // 其余日志转 stderr，供宿主诊断。
    process.stderr.write(`[wechat-sidecar][log] ${text}\n`);
  };
}

async function ensureInit(stateDir) {
  if (clientReady) return client;
  if (!initPromise) {
    initPromise = (async () => {
      const c = new WeixinClient({
        stateDir,
        log: makeLogHook(),
        errorLog: (msg) => {
          process.stderr.write(`[wechat-sidecar][errorLog] ${String(msg)}\n`);
        },
      });
      await c.init();
      c.on("message", (msg) => {
        emit({
          event: "message",
          message: {
            id: msg.id,
            accountId: msg.accountId,
            from: msg.from,
            to: msg.to,
            timestamp: msg.timestamp,
            contextToken: msg.contextToken,
            text: msg.text,
          },
        });
      });
      c.on("error", (err, accountId) => {
        emit({ event: "error", message: err?.message ?? String(err), accountId });
      });
      c.on("session_status", (status) => {
        emit({ event: "session_status", status });
      });
      // 账户快照：宿主在「重启复登（无 login_result）」场景据此恢复本人身份。
      const accounts = c.getAccounts().map((a) => ({
        id: a.id,
        name: a.name ?? null,
        configured: !!a.configured,
        userId: a.userId ?? null,
        // 会话状态供宿主在重启复登时优先选择未过期账号（跳过 session_expired）。
        status: c.getSessionStatus(a.id).status ?? null,
      }));
      emit({ event: "accounts", accounts });
      client = c;
      clientReady = true;
      emit({ event: "ready" });
      return c;
    })();
  }
  return initPromise;
}

/** 发起二维码登录：可被新登录接管或 login_cancel 取消；结果带 loginId 供宿主区分归属。 */
async function doLogin(timeoutMs, loginId) {
  if (!clientReady) {
    emit({ event: "error", message: "尚未初始化（缺少 init）", kind: "login" });
    return;
  }
  const aborter = new AbortController();
  currentLogin = { aborter, loginId };
  try {
    const result = await client.login({ force: true, timeoutMs, signal: aborter.signal });
    // 已被新登录接管/取消：丢弃结果，避免陈旧结果污染新 pending。
    if (currentLogin?.loginId !== loginId) return;
    emit({
      event: "login_result",
      success: !!result.success,
      message: result.message ?? "",
      accountId: result.account?.id,
      userId: result.account?.userId,
      loginId,
    });
  } catch (err) {
    if (currentLogin?.loginId !== loginId) return;
    emit({
      event: "login_result",
      success: false,
      message: err?.message ?? String(err),
      loginId,
    });
  } finally {
    if (currentLogin?.loginId === loginId) currentLogin = null;
  }
}

const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
rl.on("line", (raw) => {
  const line = raw.trim();
  if (!line) return;
  let cmd;
  try {
    cmd = JSON.parse(line);
  } catch (e) {
    emit({ event: "error", message: `无法解析指令: ${e.message}` });
    return;
  }
  void handleCmd(cmd);
});

async function handleCmd(cmd) {
  switch (cmd.cmd) {
    case "init": {
      try {
        await ensureInit(cmd.stateDir);
        lastStateDir = cmd.stateDir;
      } catch (err) {
        emit({
          event: "error",
          message: `初始化失败: ${err?.message ?? err}`,
          kind: "init",
        });
      }
      return;
    }
    case "login":
      // 桥端可能刚拉起 sidecar 就下发 login：先等 init 完成（幂等）再登录，
      // 新登录接管并取消上一个在途登录（避免残留阻塞后续绑定），
      // 初始化失败转为 error 事件。
      void (async () => {
        const loginId = cmd.loginId ?? `login-${(loginIdSeq += 1)}`;
        if (currentLogin) currentLogin.aborter.abort();
        try {
          if (lastStateDir) await ensureInit(lastStateDir);
          await doLogin(cmd.timeoutMs, loginId);
        } catch (err) {
          emit({
            event: "error",
            message: `登录失败: ${err?.message ?? err}`,
            kind: "login",
          });
        }
      })();
      return;
    case "login_cancel":
      if (currentLogin) currentLogin.aborter.abort();
      return;
    case "start":
      try {
        await client.start(cmd.accountId);
      } catch (err) {
        emit({ event: "error", message: `启动接收失败: ${err?.message ?? err}`, accountId: cmd.accountId });
      }
      return;
    case "stop": {
      if (!clientReady) return;
      // 带 accountId：仅停止该账号接收器（解绑单个会话用）；不带则停全部。
      if (cmd.accountId) {
        if (client.isReceiving(cmd.accountId)) {
          try {
            await client.stop(cmd.accountId);
          } catch {
            // 停止失败不影响流程
          }
        }
        return;
      }
      const accounts = clientReady ? client.getAccounts() : [];
      for (const a of accounts) {
        if (a.configured && client.isReceiving(a.id)) {
          try {
            await client.stop(a.id);
          } catch {
            // 停止失败不影响进程退出
          }
        }
      }
      return;
    }
    case "logout":
      try {
        if (cmd.accountId) await client.logout(cmd.accountId);
      } catch (err) {
        emit({ event: "error", message: `退出登录失败: ${err?.message ?? err}` });
      }
      return;
    case "send_text":
      enqueueSend(async () => {
        try {
          await client.sendText(cmd.accountId, cmd.toUserId, cmd.text, {});
        } catch (err) {
          emit({
            event: "error",
            message: `发送失败: ${err?.message ?? err}`,
            kind: "send",
            accountId: cmd.accountId,
          });
        }
      });
      return;
    default:
      emit({ event: "error", message: `未知指令: ${cmd.cmd}` });
  }
}

process.on("uncaughtException", (err) => {
  emit({ event: "error", message: `未捕获异常: ${err?.message ?? err}` });
});
emit({ event: "ready" });
