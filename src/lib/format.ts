export function formatElapsed(ms: number): string {
  const safe = Math.max(0, ms);
  const totalSeconds = Math.floor(safe / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  const tenths = Math.floor((safe % 1000) / 100);
  if (minutes >= 10) {
    return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}.${tenths}`;
}

export function formatDuration(ms: number): string {
  if (ms >= 10 * 60 * 1000) return formatElapsed(ms);
  return `${(ms / 1000).toFixed(1)}s`;
}

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

/** 毫秒时间戳 → HH:MM */
export function formatTimeHM(ms: number): string {
  const d = new Date(ms);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

/** 消息时间：与 now 同一天仅显示 HH:MM；非当天带日期（同年 M月D日，跨年补年份） */
export function formatChatTime(ms: number, now: number = Date.now()): string {
  const d = new Date(ms);
  const base = new Date(now);
  const hhmm = formatTimeHM(ms);
  const sameDay =
    d.getFullYear() === base.getFullYear() &&
    d.getMonth() === base.getMonth() &&
    d.getDate() === base.getDate();
  if (sameDay) return hhmm;
  const date =
    d.getFullYear() === base.getFullYear()
      ? `${d.getMonth() + 1}月${d.getDate()}日`
      : `${d.getFullYear()}年${d.getMonth() + 1}月${d.getDate()}日`;
  return `${date} ${hhmm}`;
}

/** 毫秒时间戳 → HH:MM:SS */
export function formatTimeHMS(ms: number): string {
  const d = new Date(ms);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
}

/** UNIX 秒时间戳 → YYYY-MM-DD HH:MM（0 返回空串） */
export function formatDateTime(secs: number): string {
  if (!secs) return "";
  const d = new Date(secs * 1000);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

export function formatRelativeTime(ts?: number | null): string {
  if (!ts) return "";
  const diff = Date.now() / 1000 - ts;
  const abs = Math.abs(diff);
  if (abs < 60) return "刚刚";
  if (abs < 3600) return `${Math.floor(abs / 60)} 分`;
  if (abs < 86400) return `${Math.floor(abs / 3600)} 小时`;
  if (abs < 86400 * 30) return `${Math.floor(abs / 86400)} 天`;
  // 超过 30 天用两位数年份的紧凑日期，保持时间列紧凑
  return new Date(ts * 1000).toLocaleDateString("zh-CN", {
    year: "2-digit",
    month: "numeric",
    day: "numeric",
  });
}

/** 把 token 数按万/亿缩写（<1万原样、>=1万一位小数万、>=1亿一位小数亿；.0 省略） */
export function formatTokens(n: number): string {
  const trim = (s: string) => s.replace(/\.0$/, "");
  if (n >= 100_000_000) return `${trim((n / 100_000_000).toFixed(1))}亿`;
  if (n >= 10_000) return `${trim((n / 10_000).toFixed(1))}万`;
  return String(n);
}

/** 取路径最后一段（兼容 / 与 \），先去除末尾分隔符；空路径返回原值 */
export function pathBaseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
}

// 路径相对化统一实现收敛到 ./path：本文件仅 re-export，保持既有调用方不变
export { relPathOf } from "./path";
