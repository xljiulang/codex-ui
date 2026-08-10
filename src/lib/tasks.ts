export interface TaskMode {
  id: "execute" | "plan" | "goal";
  label: string;
  desc: string;
  /** 24x24 SVG path（fill 风格），菜单与输入框 chip 共用 */
  icon: string;
}

export const TASK_MODES: TaskMode[] = [
  {
    id: "execute",
    label: "执行模式",
    desc: "直接执行任务并给出结果",
    icon: "M8 5v14l11-7z", // 播放/执行
  },
  {
    id: "plan",
    label: "计划模式",
    desc: "先制定计划再执行",
    icon: "M3 5h2v2H3V5zm0 6h2v2H3v-2zm0 6h2v2H3v-2zM8 5h13v2H8V5zm0 6h13v2H8v-2zm0 6h13v2H8v-2z", // 清单/计划
  },
  {
    id: "goal",
    label: "目标模式",
    desc: "设置持续追求的目标，围绕目标工作",
    icon: "M4 3h2v18H4zm2 1h13l-2.8 4 2.8 4H6z", // 旗帜/目标
  },
];

export function taskMode(id: string): TaskMode {
  return TASK_MODES.find((m) => m.id === id) ?? TASK_MODES[0];
}
