import notifyUrl from "../assets/notify.wav?url";

let lastPlayed = 0;

export function playNotificationSound(): void {
  const now = Date.now();
  if (now - lastPlayed < 800) return;
  lastPlayed = now;
  try {
    const audio = new Audio(notifyUrl);
    audio.volume = 0.7;
    void audio.play().catch(() => undefined);
  } catch {
    // 音频不可用时静默降级
  }
}

/** 仅测试用：重置节流时间戳，保证用例相互独立 */
export function __resetNotificationSoundForTest() {
  lastPlayed = 0;
}
