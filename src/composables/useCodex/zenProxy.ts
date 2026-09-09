// Zen 本地代理的设置卡片数据与命令桥（新增，独立模块）
import { invoke } from "@tauri-apps/api/core";
import { saveSettings } from "./settings";
import { setToast, toastError } from "./toast";

/** zen_proxy_status / zen_proxy_apply 命令返回的结构（与 Rust ZenProxyStatus 对齐） */
export interface ZenProxyStatus {
  running: boolean;
  port: number;
  error?: string | null;
}

const DEFAULT_PORT = 18080;

/** 查询本地代理当前状态。 */
export async function readZenProxyStatus(): Promise<ZenProxyStatus> {
  const s = await invoke<ZenProxyStatus>("zen_proxy_status");
  return s ?? { running: false, port: DEFAULT_PORT };
}

/** 应用（开启/关闭/改端口）本地代理；persist=false 时仅启停不写 settings.json。 */
export async function applyZenProxy(
  enabled: boolean,
  port: number,
  persist = true,
): Promise<ZenProxyStatus> {
  const targetPort = port > 0 ? port : DEFAULT_PORT;
  if (persist) {
    await saveSettings({ zen_proxy_enabled: enabled, zen_proxy_port: targetPort });
  }
  return invoke<ZenProxyStatus>("zen_proxy_apply", {
    enabled,
    port: targetPort,
  });
}

/** 便捷包装：应用并统一 toast 反馈；返回是否成功。 */
export async function toggleZenProxy(
  enabled: boolean,
  port: number,
): Promise<boolean> {
  try {
    const status = await applyZenProxy(enabled, port, true);
    if (status.running) {
      setToast(`Zen 本地代理已启动（端口 ${status.port}）`);
    } else {
      setToast(status.error ?? "Zen 本地代理已停止");
    }
    return status.running;
  } catch (e) {
    setToast(toastError(e));
    return false;
  }
}
