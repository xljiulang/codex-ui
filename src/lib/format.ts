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
