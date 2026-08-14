import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  __resetNotificationSoundForTest,
  playNotificationSound,
} from "../sound";

const { audioInstances } = vi.hoisted(() => ({
  audioInstances: [] as {
    volume: number;
    play: () => Promise<void>;
  }[],
}));

class MockAudio {
  volume = 0;
  play = vi.fn().mockResolvedValue(undefined);
  constructor() {
    audioInstances.push(this);
  }
}

describe("playNotificationSound 提示音", () => {
  beforeEach(() => {
    audioInstances.length = 0;
    __resetNotificationSoundForTest();
    vi.useFakeTimers({ toFake: ["Date", "setTimeout"] });
    vi.stubGlobal("Audio", MockAudio);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("播放提示音并设置音量 0.7", () => {
    playNotificationSound();
    expect(audioInstances).toHaveLength(1);
    expect(audioInstances[0].volume).toBe(0.7);
    expect(audioInstances[0].play).toHaveBeenCalled();
  });

  it("800ms 内重复触发只播放一次（节流）", () => {
    playNotificationSound();
    playNotificationSound();
    expect(audioInstances).toHaveLength(1);
    vi.advanceTimersByTime(800);
    playNotificationSound();
    expect(audioInstances).toHaveLength(2);
  });

  it("播放失败时静默降级，不抛异常", () => {
    audioInstances.length = 0;
    class ThrowingAudio {
      volume = 0;
      play = vi.fn().mockRejectedValue(new Error("no audio device"));
    }
    vi.stubGlobal("Audio", ThrowingAudio);
    expect(() => playNotificationSound()).not.toThrow();
  });
});
