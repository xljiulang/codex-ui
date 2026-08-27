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
let loggingIn = false;
/** 首次 init 的在途 Promise：并发/重复 init 幂等。 */
let initPromise = null;

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

async function doLogin(timeoutMs) {
  if (!clientReady) {
    emit({ event: "error", message: "尚未初始化（缺少 init）", kind: "login" });
    return;
  }
  if (loggingIn) return;
  loggingIn = true;
  try {
    const result = await client.login({ force: true, timeoutMs });
    emit({
      event: "login_result",
      success: !!result.success,
      message: result.message ?? "",
      accountId: result.account?.id,
      userId: result.account?.userId,
    });
  } catch (err) {
    emit({
      event: "login_result",
      success: false,
      message: err?.message ?? String(err),
    });
  } finally {
    loggingIn = false;
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
      void doLogin(cmd.timeoutMs);
      return;
    case "start":
      try {
        await client.start(cmd.accountId);
      } catch (err) {
        emit({ event: "error", message: `启动接收失败: ${err?.message ?? err}`, accountId: cmd.accountId });
      }
      return;
    case "stop": {
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
