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

/** 取路径最后一段（兼容 / 与 \），先去除末尾分隔符；空路径返回原值 */
export function pathBaseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
}

/** 路径相对 root 的表示：剥离 root 前缀（兼容 / 与 \ 分隔符），非 root 下的路径原样返回 */
export function relPathOf(root: string, path: string): string {
  const normRoot = root.replace(/[\\/]+$/, "");
  if (path.startsWith(normRoot + "\\") || path.startsWith(normRoot + "/")) {
    return path.slice(normRoot.length + 1);
  }
  return path;
}
