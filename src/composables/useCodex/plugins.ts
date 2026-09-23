// useCodex 拆分模块：设置页插件管理（plugin/list + install/uninstall + marketplace 管理）
// 与浏览器桥接自愈（chrome 插件安装后修复 latest junction / 原生宿主注册，Rust browser_bridge）。
// 协议核对：openai/codex app-server-protocol v2 schema（PluginInstallParams 等）。
import { invoke } from "@tauri-apps/api/core";
import { activeSessionTab, allSessionTabs } from "./sessionState";
import { ensureSkills, ensureThreadPlugins } from "./settings";
import type {
  PluginCatalogItem,
  PluginMarketplaceInfo,
  PluginMarketplaceLoadError,
} from "./types";
import { stripWindowsVerbatim } from "../../lib/path";

/** plugin/list 返回的原始市场/插件字段（取用子集，与服务端 schema 对齐） */
interface RawPluginSummary {
  id?: string;
  name: string;
  remotePluginId?: string | null;
  version?: string | null;
  installed?: boolean;
  enabled?: boolean;
  availability?: string;
  disabledReason?: string | null;
  authPolicy?: string;
  installPolicy?: string;
  interface?: {
    displayName?: string;
    shortDescription?: string;
    longDescription?: string;
    composerIcon?: string | null;
    composerIconUrl?: string | null;
    brandColor?: string | null;
  };
  keywords?: string[];
}

interface RawPluginListResponse {
  marketplaces?: {
    name: string;
    path?: string | null;
    interface?: { displayName?: string; shortDescription?: string };
    plugins?: RawPluginSummary[];
  }[];
  marketplaceLoadErrors?: { name?: string; error?: string }[];
}

interface PluginInstallResponse {
  authPolicy?: string;
  appsNeedingAuth?: unknown[];
}

/** 拉取插件目录：全部已配置带本地路径的市场（local）+ OpenAI 官方垂直目录（vertical） */
export async function loadPluginCatalog(force = false): Promise<{
  marketplaces: PluginMarketplaceInfo[];
  marketplaceLoadErrors: PluginMarketplaceLoadError[];
}> {
  const res = await invoke<RawPluginListResponse>("codex_rpc", {
    method: "plugin/list",
    params: {
      marketplaceKinds: ["local", "vertical"],
      forceRefetch: force,
    },
  });
  const marketplaces = (res?.marketplaces ?? []).map<PluginMarketplaceInfo>(
    (mp) => ({
      name: mp.name,
      path: mp.path ? stripWindowsVerbatim(mp.path) : null,
      isRemote: !mp.path,
      displayName: mp.interface?.displayName ?? mp.name,
      plugins: (mp.plugins ?? []).map<PluginCatalogItem>((p) => ({
        id: p.id ?? p.name,
        name: p.name,
        remotePluginId: p.remotePluginId ?? null,
        version: p.version ?? null,
        installed: p.installed ?? false,
        enabled: p.enabled ?? false,
        availability: p.availability ?? "Available",
        disabledReason: p.disabledReason ?? null,
        authPolicy: p.authPolicy ?? "",
        installPolicy: p.installPolicy ?? "",
        displayName: p.interface?.displayName ?? p.name,
        description:
          p.interface?.shortDescription ?? p.interface?.longDescription ?? "",
        iconPath: p.interface?.composerIcon ?? null,
        iconUrl: p.interface?.composerIconUrl ?? null,
        brandColor: p.interface?.brandColor ?? null,
        keywords: p.keywords ?? [],
      })),
    }),
  );
  const marketplaceLoadErrors = (res?.marketplaceLoadErrors ?? []).map((e) => ({
    name: e.name ?? "",
    error: e.error ?? "",
  }));
  return { marketplaces, marketplaceLoadErrors };
}

/** 判断安装错误是否需要账号认证（OAuth），用于给出明确中文提示 */
function isAuthRequiredError(e: unknown): boolean {
  const msg = String(e instanceof Error ? e.message : e).toLowerCase();
  return /auth|login|sign ?in|oauth|401|403/i.test(msg);
}

/** 安装插件：带本地路径的市场传 marketplacePath，远程目录市场传 remoteMarketplaceName */
export async function installPlugin(
  marketplace: PluginMarketplaceInfo,
  plugin: PluginCatalogItem,
): Promise<PluginInstallResponse> {
  const params = marketplace.isRemote
    ? {
        remoteMarketplaceName: marketplace.name,
        pluginName: plugin.name,
      }
    : {
        marketplacePath: marketplace.path,
        pluginName: plugin.name,
      };
  const res = await invoke<PluginInstallResponse>("codex_rpc", {
    method: "plugin/install",
    params,
  });
  await refreshPluginCaches();
  return res ?? {};
}

/** 卸载插件（本地或远程插件 id 均可） */
export async function uninstallPlugin(pluginId: string): Promise<void> {
  await invoke("codex_rpc", {
    method: "plugin/uninstall",
    params: { pluginId },
  });
  await refreshPluginCaches();
}

/** 添加插件市场（Git URL / owner/repo 或本地绝对路径） */
export async function addMarketplace(source: string): Promise<void> {
  await invoke("codex_rpc", {
    method: "marketplace/add",
    params: { source: stripWindowsVerbatim(source) },
  });
}

/** 移除已配置市场（同时删除其安装根） */
export async function removeMarketplace(
  marketplaceName: string,
): Promise<void> {
  await invoke("codex_rpc", {
    method: "marketplace/remove",
    params: { marketplaceName },
  });
}

/** 安装/卸载后刷新：失效全部会话的插件/技能缓存并重载活动会话，@ 菜单与技能注入即时生效 */
export async function refreshPluginCaches(): Promise<void> {
  for (const tab of allSessionTabs()) {
    tab.plugins = { plugins: [], loaded: false };
    tab.skills = { skills: [], loaded: false };
  }
  const active = activeSessionTab();
  if (active) {
    await Promise.all([ensureThreadPlugins(active), ensureSkills(active)]);
  }
}

export { isAuthRequiredError };

/** Rust browser_bridge_repair 的修复报告（camelCase） */
export interface BrowserBridgeRepairReport {
  /** ok：检查通过或修复成功；no-registration：本机无浏览器桥接注册；error：修复失败 */
  status: "ok" | "no-registration" | "error";
  /** none / already-ok / created / recreated */
  latestAction: string;
  latestTarget: string | null;
  hostPathOk: boolean;
  clientPathOk: boolean;
  registryFixed: boolean;
  message: string;
}

/** Rust browser_bridge_status 的进程状态（卸载预检用） */
export interface BrowserBridgeStatus {
  extensionHostRunning: boolean;
  nodeReplRunning: boolean;
}

/** Rust browser_bridge_stop 的结果：stopped/failed 都是进程名（failed 带原因） */
export interface BrowserBridgeStopReport {
  stopped: string[];
  failed: string[];
}

/** 是否为携带浏览器桥接的 chrome 插件（openai-bundled 的 chrome） */
export function isChromeBridgePlugin(plugin: {
  id: string;
  name: string;
}): boolean {
  return plugin.name === "chrome" || plugin.id === "chrome@openai-bundled";
}

/**
 * 浏览器桥接自愈：chrome 插件重装后 plugin/install 只恢复版本目录、不重建
 * `latest` junction，注册表引用的 `...\chrome\latest\...` 路径会断裂，导致
 * Chrome 扩展拉不起 extension-host、codex 无法控制浏览器。安装成功后调用
 * 本函数即可恢复链路；本机无浏览器桥接注册时返回 no-registration，无需处理。
 */
export async function repairBrowserBridge(): Promise<BrowserBridgeRepairReport> {
  return invoke<BrowserBridgeRepairReport>("browser_bridge_repair");
}

/** 安装/卸载预检：桥接进程（extension-host/node_repl）运行中会锁住缓存目录，装卸载将报 os error 5 */
export async function checkBrowserBridge(): Promise<BrowserBridgeStatus> {
  return invoke<BrowserBridgeStatus>("browser_bridge_status");
}

/**
 * 结束浏览器桥接进程（安装 chrome 插件前由用户确认后调用）：
 * 结束会中断当前浏览器控制会话，Chrome 扩展下次使用时会自动重新拉起 extension-host。
 */
export async function stopBrowserBridge(): Promise<BrowserBridgeStopReport> {
  return invoke<BrowserBridgeStopReport>("browser_bridge_stop");
}
