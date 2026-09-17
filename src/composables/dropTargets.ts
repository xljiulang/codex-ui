/**
 * 全局拖拽接收者注册表：ComposerBar 输入区登记自身「drop→附件」回调与矩形，
 * 由 App.vue 的单一 Tauri 拖拽监听者（useGlobalDragDrop）按 drop 坐标命中决定归属。
 *
 * 之所以需要共享注册表：Tauri onDragDropEvent 面向多个监听者直发（不走 DOM 冒泡），
 * 隐藏/非活动标签也会收到同一份事件。只有「单监听 + 坐标派发」才能保证命中
 * 当前可见输入区，且非活动标签（v-show 隐藏，矩形宽高为 0）天然被跳过。
 */
export interface DropTarget {
  /** 输入区根元素：getBoundingClientRect 判定可见性与命中 */
  el: HTMLElement;
  /** 命中的拖放路径回调（走附件逻辑） */
  onDropPaths(paths: string[]): void;
  /** 悬停高亮推入口：true 显示拖拽高亮，false 关闭 */
  setDragging(b: boolean): void;
}

const targets = new Set<DropTarget>();
/** 当前高亮目标：切换/清除时自动复位旧的 setDragging(false) */
let activeTarget: DropTarget | null = null;

/** 登记输入区为全局拖拽 drop 目标 */
export function registerDropTarget(target: DropTarget): void {
  targets.add(target);
}

/** 注销拖拽 drop 目标（组件卸载时）；若恰为当前高亮目标则一并熄灭 */
export function unregisterDropTarget(target: DropTarget): void {
  targets.delete(target);
  if (activeTarget === target) updateDropTarget(null);
}

/**
 * 按 CSS 坐标命中可见目标：
 * 仅接受宽高均 >0（隐藏/未渲染目标矩形为 0 → 跳过）且包含坐标的目标；
 * 多个命中取最后注册（视觉最上层）。
 */
export function findDropTarget(x: number, y: number): DropTarget | null {
  let hit: DropTarget | null = null;
  for (const t of targets) {
    const r = t.el.getBoundingClientRect();
    if (
      r.width > 0 &&
      r.height > 0 &&
      x >= r.left &&
      x <= r.right &&
      y >= r.top &&
      y <= r.bottom
    ) {
      hit = t;
    }
  }
  return hit;
}

/** 更新当前高亮目标：变化时先复位旧目标再点亮新目标；null 表示全部熄灭 */
export function updateDropTarget(target: DropTarget | null): void {
  if (activeTarget === target) return;
  activeTarget?.setDragging(false);
  activeTarget = target;
  target?.setDragging(true);
}
