// 探针2：请求批准模式下自动批准所有审批请求，观察哪些仍显示 declined
import { spawn } from "node:child_process";

const workdir = process.env.CODEX_PROBE_DIR;
const targetFile = process.env.CODEX_PROBE_FILE;
if (!workdir || !targetFile) {
  console.error("需要 CODEX_PROBE_DIR 和 CODEX_PROBE_FILE");
  process.exit(1);
}

const cp = spawn("codex", ["app-server", "--stdio"], {
  stdio: ["pipe", "pipe", "pipe"],
  cwd: workdir,
});

let buf = "";
let nextId = 1;
const approvals = [];
const items = [];

function send(method, params, id) {
  cp.stdin.write(
    JSON.stringify({ jsonrpc: "2.0", id: id ?? nextId++, method, params }) + "\n",
  );
}
function notify() {
  cp.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "initialized" }) + "\n");
}
function respond(id, result) {
  cp.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, result }) + "\n");
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
    if (m.id === 1 && !m.method) {
      notify();
      send(
        "thread/start",
        { cwd: workdir, approvalPolicy: "untrusted", sandbox: "read-only" },
        2,
      );
    }
    if (m.id === 2 && !m.method) {
      const prompt = `编辑文件：在 ${targetFile} 中写入两行文字 hello 和 world`;
      send(
        "turn/start",
        { threadId: m.result.thread.id, input: [{ type: "text", text: prompt, text_elements: [] }] },
        3,
      );
    }
    if (m.method && m.id !== undefined) {
      approvals.push({ method: m.method, params: m.params });
      console.log("APPROVAL:", m.method, JSON.stringify(m.params).slice(0, 700));
      let result;
      if (m.method === "item/commandExecution/requestApproval") {
        result = { decision: "accept" };
      } else if (m.method === "execCommandApproval") {
        result = { decision: "approved" };
      } else if (m.method === "item/fileChange/requestApproval") {
        result = { decision: "accept" };
      } else if (m.method === "applyPatchApproval") {
        result = { decision: "approved" };
      } else if (m.method === "item/permissions/requestApproval") {
        result = { decision: "approved" };
      } else {
        result = { action: "approve" };
      }
      respond(m.id, result);
      console.log("RESPONDED:", JSON.stringify(result));
    }
    if (m.method === "item/completed") {
      const it = m.params?.item;
      if (it && (it.type === "fileChange" || it.type === "commandExecution")) {
        items.push({ type: it.type, status: it.status, exitCode: it.exitCode ?? null });
        console.log("ITEM_COMPLETED:", it.type, "status=", it.status, "exit=", it.exitCode ?? "-");
        if (it.type === "fileChange") {
          console.log("FILECHANGE_ITEM_FULL:", JSON.stringify(it));
        }
      }
    }
    if (m.method === "item/fileChange/patchUpdated") {
      console.log("PATCH_UPDATED:", JSON.stringify(m.params).slice(0, 1200));
    }
    if (m.method === "turn/completed") {
      console.log("TURN_DONE. approvals:", approvals.length, "items:", JSON.stringify(items));
      cp.kill();
      setTimeout(() => process.exit(0), 300);
    }
  }
});

send(
  "initialize",
  {
    clientInfo: { name: "probe2", title: "probe2", version: "0.1.0" },
    capabilities: { experimentalApi: true, requestAttestation: false },
  },
  1,
);

setTimeout(() => {
  console.log("TIMEOUT approvals:", approvals.length, "items:", JSON.stringify(items));
  cp.kill();
  process.exit(0);
}, 180000);
