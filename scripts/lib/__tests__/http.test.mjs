// `scripts/lib/http.mjs` 的单元测试：用本地假源站 + 假 CONNECT 代理验证
// 「有代理走隧道、命中 NO_PROXY 直连」的全部行为，不联网、不碰 TLS 证书。
//
// 运行：npm run test:scripts（等价 node --test scripts）

import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import http from "node:http";
import net from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, describe, it } from "node:test";

import { downloadToFile, readText, resolveProxy } from "../http.mjs";

/** 监听随机端口并返回 `{ port, url, server }`。 */
async function listen(server) {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  return { port, url: `http://127.0.0.1:${port}`, server };
}

function close(server) {
  return new Promise((resolve) => server.close(resolve));
}

/** 假 CONNECT 代理：记录每条 CONNECT 隧道，可切换成拒绝握手。 */
function makeProxy() {
  const connects = [];
  let mode = "tunnel";
  const sockets = new Set();
  const server = http.createServer((_req, res) => {
    res.writeHead(405).end();
  });
  server.on("connect", (req, clientSocket, head) => {
    connects.push(req.url);
    sockets.add(clientSocket);
    clientSocket.on("close", () => sockets.delete(clientSocket));
    if (mode === "deny") {
      clientSocket.end("HTTP/1.1 403 Forbidden\r\n\r\n");
      return;
    }
    const [host, port] = req.url.split(":");
    const upstream = net.connect(Number(port), host, () => {
      clientSocket.write("HTTP/1.1 200 Connection Established\r\n\r\n");
      if (head?.length) upstream.write(head);
      upstream.pipe(clientSocket);
      clientSocket.pipe(upstream);
    });
    upstream.on("error", () => clientSocket.destroy());
  });
  return {
    server,
    connects,
    setMode(next) {
      mode = next;
    },
    destroySockets() {
      for (const socket of sockets) socket.destroy();
    },
  };
}

let workDir;
let origin;
let originUrl;
let proxy;
let proxyUrl;

before(async () => {
  workDir = await mkdtemp(join(tmpdir(), "codex-http-test-"));

  origin = http.createServer((req, res) => {
    const path = new URL(req.url, "http://127.0.0.1").pathname;
    if (path === "/redirect") {
      res.writeHead(302, { Location: "/ok" }).end();
      return;
    }
    if (path === "/loop") {
      res.writeHead(302, { Location: "/loop" }).end();
      return;
    }
    if (path === "/missing") {
      res.writeHead(404).end("nope");
      return;
    }
    if (path === "/big") {
      res.writeHead(200, { "Content-Type": "application/octet-stream" });
      res.end(Buffer.alloc(256 * 1024, 7));
      return;
    }
    res.writeHead(200, { "Content-Type": "text/plain" }).end("hello-body");
  });
  ({ url: originUrl } = await listen(origin));

  proxy = makeProxy();
  ({ url: proxyUrl } = await listen(proxy.server));
});

after(async () => {
  proxy.destroySockets();
  await close(proxy.server);
  await close(origin);
  await rm(workDir, { recursive: true, force: true });
});

/** 只在本次调用里生效的环境变量集合（避免污染 process.env）。 */
function env(pairs) {
  return { ...pairs };
}

describe("resolveProxy", () => {
  it("无代理变量时直连", () => {
    assert.deepEqual(resolveProxy("https://example.com/a", env({})), {
      proxy: null,
    });
  });

  it("https 目标读 HTTPS_PROXY，小写变体同样生效", () => {
    assert.equal(
      resolveProxy("https://example.com/a", env({ HTTPS_PROXY: "p:1" })).proxy,
      "p:1",
    );
    assert.equal(
      resolveProxy("https://example.com/a", env({ https_proxy: "p:2" })).proxy,
      "p:2",
    );
  });

  it("http 目标读 HTTP_PROXY，且不回落到 HTTPS_PROXY", () => {
    assert.equal(
      resolveProxy("http://example.com/a", env({ HTTP_PROXY: "p:3" })).proxy,
      "p:3",
    );
    assert.equal(
      resolveProxy("http://example.com/a", env({ HTTPS_PROXY: "p:4" })).proxy,
      null,
    );
  });

  it("目标协议对应的变量缺失时回退 ALL_PROXY", () => {
    assert.equal(
      resolveProxy("https://example.com/a", env({ ALL_PROXY: "p:5" })).proxy,
      "p:5",
    );
    assert.equal(
      resolveProxy("http://example.com/a", env({ all_proxy: "p:6" })).proxy,
      "p:6",
    );
  });

  it("空白值按未设置处理", () => {
    assert.equal(
      resolveProxy("https://example.com/a", env({ HTTPS_PROXY: "   " })).proxy,
      null,
    );
  });

  it("NO_PROXY=* 一律直连", () => {
    assert.equal(
      resolveProxy(
        "https://example.com/a",
        env({ HTTPS_PROXY: "p:7", NO_PROXY: "*" }),
      ).proxy,
      null,
    );
  });

  it("NO_PROXY 支持精确域名、前导点与子域后缀", () => {
    const base = { HTTPS_PROXY: "p:8" };
    assert.equal(
      resolveProxy(
        "https://github.com/a",
        env({ ...base, NO_PROXY: "github.com" }),
      ).proxy,
      null,
    );
    assert.equal(
      resolveProxy(
        "https://api.github.com/a",
        env({ ...base, NO_PROXY: ".github.com" }),
      ).proxy,
      null,
    );
    assert.equal(
      resolveProxy(
        "https://github.com.evil.test/a",
        env({ ...base, NO_PROXY: "github.com" }),
      ).proxy,
      "p:8",
    );
  });

  it("NO_PROXY 域名大小写不敏感、支持逗号分隔多条目", () => {
    assert.equal(
      resolveProxy(
        "https://API.GitHub.com/a",
        env({ HTTPS_PROXY: "p:9", NO_PROXY: "example.com, GitHub.com " }),
      ).proxy,
      null,
    );
  });

  it("NO_PROXY 带端口时端口必须一致", () => {
    const base = { HTTPS_PROXY: "p:10" };
    assert.equal(
      resolveProxy(
        "https://example.com/a",
        env({ ...base, NO_PROXY: "example.com:443" }),
      ).proxy,
      null,
    );
    assert.equal(
      resolveProxy(
        "https://example.com/a",
        env({ ...base, NO_PROXY: "example.com:8443" }),
      ).proxy,
      "p:10",
    );
  });
});

describe("downloadToFile", () => {
  it("无代理时直连下载，字节与源站一致", async () => {
    const dest = join(workDir, "direct.bin");
    const before = proxy.connects.length;
    const result = await downloadToFile(`${originUrl}/big`, dest, {
      env: env({ HTTPS_PROXY: proxyUrl }),
    });
    assert.equal(result.bytes, 256 * 1024);
    assert.equal((await readFile(dest)).length, 256 * 1024);
    assert.equal(
      proxy.connects.length,
      before,
      "http 目标未配置 http_proxy 时不应走代理",
    );
  });

  it("配置代理时经 CONNECT 隧道下载，字节与源站一致", async () => {
    const dest = join(workDir, "proxied.bin");
    const before = proxy.connects.length;
    const routes = [];
    const result = await downloadToFile(`${originUrl}/big`, dest, {
      env: env({ HTTP_PROXY: proxyUrl }),
      onRoute: (route) => routes.push(route),
    });
    assert.equal(result.bytes, 256 * 1024);
    assert.deepEqual((await readFile(dest)).subarray(0, 4), Buffer.alloc(4, 7));
    assert.equal(proxy.connects.length, before + 1);
    assert.equal(proxy.connects.at(-1), `127.0.0.1:${new URL(originUrl).port}`);
    assert.equal(routes.length, 1);
    assert.equal(routes[0].proxy, proxyUrl);
    assert.equal(routes[0].finalUrl, `${originUrl}/big`);
  });

  it("命中 NO_PROXY 时跳过代理直连", async () => {
    const dest = join(workDir, "no-proxy.bin");
    const before = proxy.connects.length;
    await downloadToFile(`${originUrl}/ok`, dest, {
      env: env({ HTTP_PROXY: proxyUrl, NO_PROXY: "127.0.0.1" }),
    });
    assert.equal(proxy.connects.length, before, "命中 NO_PROXY 后不应再建隧道");
    assert.equal((await readFile(dest)).toString(), "hello-body");
  });

  it("onRoute 在写盘前回调，且此时目标文件尚未创建", async () => {
    const dest = join(workDir, "route-order.bin");
    let destExisted = null;
    await downloadToFile(`${originUrl}/ok`, dest, {
      env: env({ HTTP_PROXY: proxyUrl }),
      onRoute: async () => {
        destExisted = await stat(dest).then(
          () => true,
          () => false,
        );
      },
    });
    assert.equal(destExisted, false);
  });

  it("自动跟随 302 并把最终内容落盘", async () => {
    const dest = join(workDir, "redirect.txt");
    const result = await downloadToFile(`${originUrl}/redirect`, dest, {
      env: env({ HTTP_PROXY: proxyUrl }),
    });
    assert.equal(result.finalUrl, `${originUrl}/ok`);
    assert.equal((await readFile(dest)).toString(), "hello-body");
  });

  it("重定向超过上限时报错且不落盘", async () => {
    const dest = join(workDir, "loop.txt");
    await assert.rejects(
      () => downloadToFile(`${originUrl}/loop`, dest, { env: env({}) }),
      /重定向次数过多/,
    );
    assert.equal(
      await stat(dest).then(
        () => true,
        () => false,
      ),
      false,
    );
  });

  it("非 2xx 直接抛错", async () => {
    await assert.rejects(
      () =>
        downloadToFile(`${originUrl}/missing`, join(workDir, "404.bin"), {
          env: env({}),
        }),
      /HTTP 404/,
    );
  });

  it("代理拒绝 CONNECT 时错误信息带状态行", async () => {
    proxy.setMode("deny");
    try {
      await assert.rejects(
        () =>
          downloadToFile(`${originUrl}/ok`, join(workDir, "denied.bin"), {
            env: env({ HTTP_PROXY: proxyUrl }),
          }),
        /CONNECT .* 失败: HTTP\/1\.1 403 Forbidden/,
      );
    } finally {
      proxy.setMode("tunnel");
    }
  });

  it("代理不可达时错误信息带代理地址", async () => {
    const dead = await listen(http.createServer());
    await close(dead.server);
    await assert.rejects(
      () =>
        downloadToFile(`${originUrl}/ok`, join(workDir, "dead.bin"), {
          env: env({ HTTP_PROXY: dead.url }),
        }),
      (error) => {
        assert.match(error.message, /^代理连接失败: /);
        return true;
      },
    );
  });

  it("不支持的代理协议（socks5）明确报错，不退化成直连", async () => {
    await assert.rejects(
      () =>
        downloadToFile(`${originUrl}/ok`, join(workDir, "socks.bin"), {
          env: env({ HTTP_PROXY: "socks5://127.0.0.1:1080" }),
        }),
      /不支持的代理协议 socks5:\/\//,
    );
  });
});

describe("readText", () => {
  it("经代理读取文本并返回状态码", async () => {
    const result = await readText(`${originUrl}/ok`, {
      env: env({ HTTP_PROXY: proxyUrl }),
    });
    assert.equal(result.status, 200);
    assert.equal(result.text, "hello-body");
  });

  it("非 2xx 不抛错，只把状态码交给调用方", async () => {
    const result = await readText(`${originUrl}/missing`, { env: env({}) });
    assert.equal(result.status, 404);
    assert.equal(result.text, "nope");
  });
});
