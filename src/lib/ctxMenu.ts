/**
 * 右键菜单定位：把菜单完整收进视口。
 * 视口足够大时保持点击位置；菜单超出右侧/底部时左移/上移；
 * 窗口比菜单还小时贴边显示（不小于 pad），避免坐标为负导致菜单不可见。
 */
export function clampMenuPos(
  x: number,
  y: number,
  width: number,
  height: number,
  pad = 4,
): { x: number; y: number } {
  const maxX = Math.max(pad, window.innerWidth - width - pad);
  const maxY = Math.max(pad, window.innerHeight - height - pad);
  return {
    x: Math.min(Math.max(pad, x), maxX),
    y: Math.min(Math.max(pad, y), maxY),
  };
}
