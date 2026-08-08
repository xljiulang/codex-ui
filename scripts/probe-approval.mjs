// 开发探针：验证 codex app-server 审批请求与响应格式（实测用，非产品代码）
import { spawn } from "node:child_process";

const workdir = process.env.CODEX_PROBE_DIR;
if (!workdir) {
  console.error("CODEX_PROBE_DIR 未设置");
  process.exit(1);
}

const cp = spawn("codex", ["app-server", "--stdio"], {
  stdio: ["pipe", "pipe", "pipe"],
  cwd: workdir,
});

let buf = "";
let nextId = 1;
const events = [];

function send(method, params, id) {
  cp.stdin.write(
    JSON.stringify({ jsonrpc: "2.0", id: id ?? nextId++, method, params }) + "\n",
  );
}

function notify(method, params) {
  cp.stdin.write(JSON.stringify({ jsonrpc: "2.0", method, params }) + "\n");
}

cp.stdout.on("data", (d) => {
  buf += d.toString("utf8");
  let i;
  while ((i = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, i).trim();
    buf = buf.slice(i + 1);
    if (!line) continue;
    let m;
    try {
      m = JSON.parse(line);
    } catch {
      continue;
    }
    if (m.id === 1) {
      notify("initialized");
      send(
        "thread/start",
        {
          cwd: workdir,
          approvalPolicy: "untrusted",
          sandbox: "read-only",
        },
        2,
      );
    }
    if (m.id === 2) {
      console.log("THREAD_START_RESP", JSON.stringify(m).slice(0, 600));
    }
    if (m.id === 2) {
      send(
        "turn/start",
        {
          threadId: m.result.thread.id,
          input: [{ type: "text", text: "运行 shell 命令: whoami", text_elements: [] }],
        },
        3,
      );
    }
    if (m.id === 3) {
      console.log("TURN_START_RESP", JSON.stringify(m.result).slice(0, 300));
    }
    if (m.method) {
      events.push(m.method);
      if (m.method === "item/completed") {
        const it = m.params?.item;
        if (it?.type === "commandExecution") {
          console.log(
            "CMD_STATUS:",
            it.status,
            "exit=",
            it.exitCode ?? "-",
          );
        }
      }
      if (
        m.method === "item/commandExecution/requestApproval" ||
        m.method === "execCommandApproval"
      ) {
        console.log("APPROVAL_METHOD", m.method);
        console.log("APPROVAL_PARAMS", JSON.stringify(m.params).slice(0, 1600));
        const proposed = m.params?.proposedExecpolicyAmendment;
        cp.stdin.write(
          JSON.stringify({
            jsonrpc: "2.0",
            id: m.id,
            result: {
              decision: proposed
                ? {
                    acceptWithExecpolicyAmendment: {
                      execpolicy_amendment: proposed,
                    },
                  }
                : "accept",
            },
          }) + "\n",
        );
        console.log("RESPONDED amendment:", JSON.stringify(proposed));
      }
      if (m.method === "turn/completed") {
        console.log("TURN_COMPLETED");
        console.log("EVENTS", events.join(" "));
        cp.kill();
        setTimeout(() => process.exit(0), 500);
      }
    }
  }
});

cp.stderr.on("data", (d) => {
  const s = d.toString();
  if (s.includes("ERROR")) console.log("STDERR", s.trim().slice(0, 300));
});

send(
  "initialize",
  {
    clientInfo: { name: "codex-ui", title: "Codex UI", version: "0.1.0" },
    capabilities: { experimentalApi: true, requestAttestation: false },
  },
  1,
);

setTimeout(() => {
  console.log("TIMEOUT");
  console.log("EVENTS", events.join(" "));
  cp.kill();
  process.exit(0);
}, 120000);
