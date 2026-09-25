#!/usr/bin/env node
// 更新安装包内置二进制（setup/bin/）：按固定版本下载 → 校验 sha256 → 落盘。
//
//   node scripts/update-setup-bin.mjs           # 下载并替换 setup/bin 下的 5 个二进制
//   node scripts/update-setup-bin.mjs --check   # 只校验现值是否等于钉死哈希（不联网）
//
// 这些文件被 .gitignore 忽略（`/setup/bin/*.exe`），不进版本库；入库的是本脚本、
// 下面的哈希表与文档。setup/setup.iss 用 `.\bin\*` 平铺安装，文件名必须保持不变。
//
// 组成：codex 主程序 + 3 个 helper 取自 openai/codex 的 Release 资产；rg.exe 取自
// BurntSushi/ripgrep 的官方预编译包（与 codex npm 包内 `codex-path/rg.exe` 字节相同，
// 但只需下载约 1.7 MB，不必拉整个 codex-package 压缩包）。

import { createHash } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readdir,
  rename,
  rm,
  stat,
} from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const BIN_DIR = join(ROOT, "setup", "bin");

/** codex 版本：与仓库验证基线（README「协议与版本支持」）保持一致。 */
const CODEX_VERSION = "0.156.1";
/** ripgrep 版本：与 codex 包内 `codex-path/rg.exe` 同版本。 */
const RIPGREP_VERSION = "15.2.0";

const CODEX_ASSETS = `https://github.com/openai/codex/releases/download/rust-v${CODEX_VERSION}`;
const RIPGREP_ASSETS = `https://github.com/BurntSushi/ripgrep/releases/download/${RIPGREP_VERSION}`;
const RIPGREP_ZIP = `${RIPGREP_ASSETS}/ripgrep-${RIPGREP_VERSION}-x86_64-pc-windows-msvc.zip`;

/**
 * 期望落盘的 5 个文件。`codexAsset` 为 openai/codex Release 资产名；
 * `zipEntry` 表示该文件要从压缩包里取（而非直接下载）。
 * sha256 与字节数取自官方发布物实测，任一不符即中止且不改动 setup/bin。
 */
const FILES = [
  {
    name: "codex.exe",
    codexAsset: "codex-x86_64-pc-windows-msvc.exe",
    sha256: "70bcb05f9bf1a4e7306edd0cd1b57d02af3267ad02a34b26f45c8c4bb20a3301",
    bytes: 323383088,
  },
  {
    name: "codex-code-mode-host.exe",
    codexAsset: "codex-code-mode-host-x86_64-pc-windows-msvc.exe",
    sha256: "0f83a73dc6d511d43bd3e52cc0a999cb383c19c645ef3fbd8fbdaddde3088138",
    bytes: 72466736,
  },
  {
    name: "codex-command-runner.exe",
    codexAsset: "codex-command-runner-x86_64-pc-windows-msvc.exe",
    sha256: "67fde4e4983c031675773f632991f14794328a8cccf2bc542137bd2658370822",
    bytes: 8222000,
  },
  {
    name: "codex-windows-sandbox-setup.exe",
    codexAsset: "codex-windows-sandbox-setup-x86_64-pc-windows-msvc.exe",
    sha256: "2bb0e7bd8ae23757c5273c4e82b506eb2923ed5c75facaef230934a1e0aab87b",
    bytes: 15472944,
  },
  // rg 从 zip 里取：官方 zip 内的 rg.exe 即目标文件
  {
    name: "rg.exe",
    zipUrl: RIPGREP_ZIP,
    zipEntry: "rg.exe",
    sha256: "14231169855ec5205cf5a1b6f1db358ff4aed4247c86b69ce8aae647c77f6680",
    bytes: 4218880,
  },
];

const checkOnly = process.argv.slice(2).includes("--check");

/** 下载重试次数：GitHub 大文件偶发中途断开，重试即可（每次重下同一文件）。 */
const DOWNLOAD_ATTEMPTS = 3;

/** 逐个流式下载到目标路径（大文件不整块读入内存），失败按次数重试。 */
async function download(url, dest) {
  for (let attempt = 1; ; attempt += 1) {
    console.log(`下载 ${url}（第 ${attempt}/${DOWNLOAD_ATTEMPTS} 次）`);
    try {
      const response = await fetch(url, { redirect: "follow" });
      if (!response.ok || !response.body) {
        throw new Error(`HTTP ${response.status} ${response.statusText}`);
      }
      await pipeline(Readable.fromWeb(response.body), createWriteStream(dest));
      return;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      await rm(dest, { force: true });
      if (attempt >= DOWNLOAD_ATTEMPTS) {
        throw new Error(
          `下载失败（已重试 ${DOWNLOAD_ATTEMPTS} 次）: ${url} — ${message}`,
        );
      }
      console.warn(`下载中断，重试：${message}`);
    }
  }
}

/** 解出 zip 内指定 basename 的条目到 dest（Windows 自带 tar.exe 读 zip）。 */
async function extractFromZip(zipPath, entryName, dest, workDir) {
  const unpackDir = join(workDir, "unzip");
  await mkdir(unpackDir, { recursive: true });
  try {
    execFileSync("tar", ["-xf", zipPath, "-C", unpackDir], {
      stdio: "inherit",
    });
  } catch {
    // 极少数环境没有 tar.exe：退回 PowerShell（Windows 10/11 自带）
    execFileSync(
      "powershell",
      [
        "-NoProfile",
        "-Command",
        `Expand-Archive -LiteralPath '${zipPath}' -DestinationPath '${unpackDir}' -Force`,
      ],
      { stdio: "inherit" },
    );
  }
  const found = [];
  const walk = async (dir) => {
    for (const item of await readdir(dir, { withFileTypes: true })) {
      const full = join(dir, item.name);
      if (item.isDirectory()) await walk(full);
      else if (item.name.toLowerCase() === entryName.toLowerCase())
        found.push(full);
    }
  };
  await walk(unpackDir);
  if (found.length !== 1) {
    throw new Error(
      `压缩包内 ${entryName} 命中 ${found.length} 个（期望 1 个）`,
    );
  }
  await rename(found[0], dest);
}

/** 流式计算 sha256（codex.exe 有 300+ MB，不整块读入内存）。 */
async function sha256Of(path) {
  const hash = createHash("sha256");
  await pipeline(createReadStream(path), hash);
  return hash.digest("hex");
}

/** 校验单个已落盘文件；返回实际 sha256，不符时抛错。 */
async function verify(path, entry) {
  let info;
  try {
    info = await stat(path);
  } catch {
    throw new Error("文件不存在（全新 clone 或未下载过）");
  }
  if (info.size !== entry.bytes) {
    throw new Error(
      `${entry.name} 字节数不符：实际 ${info.size}，期望 ${entry.bytes}`,
    );
  }
  const actual = await sha256Of(path);
  if (actual !== entry.sha256) {
    throw new Error(
      `${entry.name} sha256 不符：实际 ${actual}，期望 ${entry.sha256}`,
    );
  }
  return actual;
}

/** --check：只对账 setup/bin 现值，不联网、不写盘。 */
async function runCheck() {
  const bad = [];
  for (const entry of FILES) {
    const path = join(BIN_DIR, entry.name);
    try {
      await verify(path, entry);
      console.log(`OK   ${entry.name}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.log(`FAIL ${entry.name} — ${message}`);
      bad.push(entry.name);
    }
  }
  if (bad.length > 0) {
    throw new Error(
      `setup/bin 有 ${bad.length} 个文件与 codex-cli ${CODEX_VERSION} 不符：${bad.join(", ")}；` +
        "运行 node scripts/update-setup-bin.mjs 更新",
    );
  }
  console.log(
    `setup/bin 全部匹配 codex-cli ${CODEX_VERSION} + ripgrep ${RIPGREP_VERSION}`,
  );
}

/** 下载 → 校验 → 全部通过后一次性覆盖 setup/bin。 */
async function runUpdate() {
  const workDir = await mkdtemp(join(tmpdir(), "codex-setup-bin-"));
  try {
    const staged = [];
    for (const entry of FILES) {
      const dest = join(workDir, entry.name);
      if (entry.zipEntry) {
        const zipPath = join(workDir, "download.zip");
        await download(entry.zipUrl, zipPath);
        await extractFromZip(zipPath, entry.zipEntry, dest, workDir);
      } else {
        await download(`${CODEX_ASSETS}/${entry.codexAsset}`, dest);
      }
      const actual = await verify(dest, entry);
      console.log(`校验通过 ${entry.name} sha256=${actual}`);
      staged.push({ entry, dest });
    }

    await mkdir(BIN_DIR, { recursive: true });
    for (const { entry, dest } of staged) {
      // 临时目录在 C:、仓库可能在别的盘：用 copy 而非 rename，避免跨卷失败
      await copyFile(dest, join(BIN_DIR, entry.name));
    }
    await runCheck();
  } finally {
    await rm(workDir, { recursive: true, force: true });
  }
}

(checkOnly ? runCheck() : runUpdate()).catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
