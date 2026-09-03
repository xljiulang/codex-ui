import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  applyGlassEffect,
  applyTheme,
  loadTheme,
  normalizeTheme,
  previewTheme,
} from "../useTheme";

const { storage } = vi.hoisted(() => ({
  storage: new Map<string, string>(),
}));

describe("useTheme 主题管理", () => {
  beforeEach(() => {
    storage.clear();
    vi.stubGlobal("localStorage", {
      getItem: (k: string) => storage.get(k) ?? null,
      setItem: (k: string, v: string) => {
        storage.set(k, String(v));
      },
      removeItem: (k: string) => {
        storage.delete(k);
      },
      clear: () => storage.clear(),
      key: () => null,
      get length() {
        return storage.size;
      },
    });
    delete document.documentElement.dataset.theme;
  });

  afterEach(() => {
    storage.clear();
    vi.unstubAllGlobals();
    delete document.documentElement.dataset.theme;
  });

  it("normalizeTheme 非法值回退 blue", () => {
    expect(normalizeTheme("blue")).toBe("blue");
    expect(normalizeTheme("dark")).toBe("dark");
    expect(normalizeTheme("light")).toBe("light");
    expect(normalizeTheme(null)).toBe("blue");
    expect(normalizeTheme(undefined)).toBe("blue");
    expect(normalizeTheme("unknown")).toBe("blue");
  });

  it("applyTheme 写入根节点 data-theme 与本地缓存", () => {
    expect(applyTheme("dark")).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(localStorage.getItem("codex-ui-theme")).toBe("dark");
  });

  it("loadTheme 从本地缓存读取，缺失或非法时回退 blue", () => {
    expect(loadTheme()).toBe("blue");
    localStorage.setItem("codex-ui-theme", "light");
    expect(loadTheme()).toBe("light");
    localStorage.setItem("codex-ui-theme", "nope");
    expect(loadTheme()).toBe("blue");
  });

  it("previewTheme 仅即时切换，不写本地缓存（取消时还原）", () => {
    applyTheme("blue");
    expect(previewTheme("light")).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(localStorage.getItem("codex-ui-theme")).toBe("blue");
  });

  it("applyGlassEffect 写入根节点 data-glass", () => {
    applyGlassEffect(true);
    expect(document.documentElement.dataset.glass).toBe("on");
    applyGlassEffect(false);
    expect(document.documentElement.dataset.glass).toBe("off");
  });
});
