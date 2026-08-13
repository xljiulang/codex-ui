import { invoke } from "@tauri-apps/api/core";
import { resolveCwd, setToast, toastError } from "../composables/useCodex";

export type LinkClassification =
  | { kind: "web"; url: string }
  | { kind: "local"; path: string }
  | null;

/** 把 Windows 风格路径统一为反斜杠 */
function toBackslash(p: string): string {
  return p.replace(/\//g, "\\");
}

/** 简单 Windows 路径解析：处理 . 与 ..，不做文件系统访问 */
function resolveWindowsPath(base: string, rel: string): string {
  const norm = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "");
  const parts = [...norm(base).split("/"), ...norm(rel).split("/")];
  const out: string[] = [];
  for (const part of parts) {
    if (!part || part === ".") continue;
    if (part === "..") out.pop();
    else out.push(part);
  }
  return toBackslash(out.join("/"));
}

/**
 * 分类对话内容中的链接：
 * - http/https → 网页（外置浏览器）；
 * - file:///、/X:/、X:/、X:\ → 本地路径（解码并统一为反斜杠）；
 * - 其它相对路径（排除 mailto:/#/javascript: 等）→ 按工作目录解析为本地路径；
 * - 无法分类返回 null（点击无效）。
 */
export function localPathFromHref(
  href: string,
  workspaceRoot: string,
): LinkClassification {
  let h = href.trim();
  if (!h) return null;
  try {
    h = decodeURIComponent(h);
  } catch {
    // 保留原值
  }
  if (/^https?:\/\//i.test(h)) return { kind: "web", url: h };

  let local = h;
  if (local.startsWith("file:///")) local = local.slice("file:///".length);
  if (/^\/[A-Za-z]:\//.test(local)) local = local.slice(1);
  if (/^[A-Za-z]:[\\/]/.test(local)) {
    return { kind: "local", path: toBackslash(local) };
  }
  if (local.startsWith("#")) return null;
  // 其它 scheme（plugin://、mailto:、javascript: 等）不作为本地路径，避免误解析为相对路径
  if (/^[A-Za-z][A-Za-z0-9+.-]*:/.test(local)) return null;
  if (workspaceRoot) {
    return { kind: "local", path: resolveWindowsPath(workspaceRoot, local) };
  }
  return null;
}

export function displayHref(href: string, workspaceRoot: string): string {
  const cls = localPathFromHref(href, workspaceRoot);
  if (!cls) return href;
  return cls.kind === "web" ? cls.url : cls.path;
}

export function workspaceRoot(): string {
  return resolveCwd();
}

interface TestHookWindow {
  __CODEX_UI_TEST__?: boolean;
  __CODEX_UI_TEST_LOG__?: { cmd: string; args: Record<string, unknown> }[];
}

/** 网页走默认浏览器；本地路径用资源管理器定位（文件 /select，目录打开），失败 toast 提示 */
export function openLink(href: string, root: string = workspaceRoot()) {
  const cls = localPathFromHref(href, root);
  if (!cls) return;
  const testWin = window as unknown as TestHookWindow;
  // E2E 测试钩子：__CODEX_UI_TEST__ 开启时只记录分发，不真正打开浏览器/资源管理器
  if (testWin.__CODEX_UI_TEST__) {
    const log = (testWin.__CODEX_UI_TEST_LOG__ ??= []);
    log.push(
      cls.kind === "web"
        ? { cmd: "open_url", args: { url: cls.url } }
        : { cmd: "reveal_path", args: { path: cls.path } },
    );
    return;
  }
  if (cls.kind === "web") {
    void invoke("open_url", { url: cls.url }).catch(() => undefined);
    return;
  }
  void invoke("reveal_path", { path: cls.path }).catch((e) => {
    setToast(toastError(e));
  });
}
