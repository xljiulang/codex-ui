// 兼容代理的设置卡片数据与命令桥（新增，独立模块）
import { invoke } from "@tauri-apps/api/core";
import { saveSettings } from "./settings";
import { setToast, toastError } from "./toast";

/** compat_proxy_status / compat_proxy_apply 命令返回的结构（与 Rust CompatProxyStatus 对齐） */
export interface CompatProxyStatus {
  running: boolean;
  port: number;
  error?: string | null;
}

const DEFAULT_PORT = 18080;
const DEFAULT_BASE_URL = "https://opencode.ai/zen/v1";

/** 应用本地代理所需的全部参数（两个行为开关缺省按开启）。 */
export interface CompatProxyOptions {
  enabled: boolean;
  port: number;
  baseUrl: string;
  /** 「回合收尾约束和助推」：空转收尾时自动续跑，默认开启 */
  nudgeEnabled?: boolean;
  /** 「OpenCode 客户端身份」：识别头/UA + 免费层门禁补丁（不分上游），默认开启 */
  identityEnabled?: boolean;
}

/** 查询本地代理当前状态。 */
export async function readCompatProxyStatus(): Promise<CompatProxyStatus> {
  const s = await invoke<CompatProxyStatus>("compat_proxy_status");
  return s ?? { running: false, port: DEFAULT_PORT };
}

/** 应用（开启/关闭/改端口/改行为开关）本地代理；persist=false 时仅启停不写 settings.json。 */
export async function applyCompatProxy(
  options: CompatProxyOptions,
  persist = true,
): Promise<CompatProxyStatus> {
  const { enabled } = options;
  const targetPort = options.port > 0 ? options.port : DEFAULT_PORT;
  const targetBaseUrl =
    options.baseUrl.trim() === "" ? DEFAULT_BASE_URL : options.baseUrl.trim();
  const nudgeEnabled = options.nudgeEnabled ?? true;
  const identityEnabled = options.identityEnabled ?? true;
  if (persist) {
    await saveSettings({
      compat_proxy_enabled: enabled,
      compat_proxy_port: targetPort,
      compat_proxy_base_url: targetBaseUrl,
      compat_proxy_nudge_enabled: nudgeEnabled,
      compat_proxy_identity_enabled: identityEnabled,
    });
  }
  return invoke<CompatProxyStatus>("compat_proxy_apply", {
    enabled,
    port: targetPort,
    baseUrl: targetBaseUrl,
    nudgeEnabled,
    opencodeIdentityEnabled: identityEnabled,
  });
}

/** 便捷包装：应用并统一 toast 反馈；返回是否成功。 */
export async function toggleCompatProxy(options: CompatProxyOptions): Promise<boolean> {
  try {
    const status = await applyCompatProxy(options, true);
    if (status.running) {
      setToast(`兼容代理已启动（端口 ${status.port}）`);
    } else {
      setToast(status.error ?? "兼容代理已停止");
    }
    return status.running;
  } catch (e) {
    setToast(toastError(e));
    return false;
  }
}
