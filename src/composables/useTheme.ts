import type { ThemeId } from "../lib/types";

export type { ThemeId };

export interface ThemeOption {
  id: ThemeId;
  name: string;
  desc: string;
}

export const THEMES: ThemeOption[] = [
  { id: "blue", name: "蓝夜", desc: "默认蓝色风格" },
  { id: "dark", name: "曜黑", desc: "近黑画布 · 霓虹紫" },
  { id: "light", name: "晨光", desc: "通透明亮" },
];

const THEME_KEY = "codex-ui-theme";

export function normalizeTheme(theme: string | null | undefined): ThemeId {
  return THEMES.some((t) => t.id === theme) ? (theme as ThemeId) : "blue";
}

/** 启动时从本地缓存读取主题（与设置文件双写，避免启动闪屏） */
export function loadTheme(): ThemeId {
  try {
    return normalizeTheme(localStorage.getItem(THEME_KEY));
  } catch {
    return "blue";
  }
}

/** 应用主题：写入根节点 data-theme 并同步本地缓存 */
export function applyTheme(theme: string | null | undefined): ThemeId {
  const id = normalizeTheme(theme);
  document.documentElement.dataset.theme = id;
  try {
    localStorage.setItem(THEME_KEY, id);
  } catch {
    // 无 localStorage（隐私模式/受限环境）时仅即时生效
  }
  return id;
}

/** 预览主题：仅即时切换根节点 data-theme，不写入本地缓存（取消时还原） */
export function previewTheme(theme: string | null | undefined): ThemeId {
  const id = normalizeTheme(theme);
  document.documentElement.dataset.theme = id;
  return id;
}

/** 应用毛玻璃特效开关：写入根节点 data-glass（CSS 据此切换透明根背景） */
export function applyGlassEffect(enabled: boolean) {
  document.documentElement.dataset.glass = enabled ? "on" : "off";
}
